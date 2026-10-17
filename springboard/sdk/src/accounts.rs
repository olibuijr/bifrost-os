//! Accounts (`ACAccountStore`, `ACAccount`, `ACAccountType`, `ACAccountCredential`, `ACErrorCode`): the user's
//! external accounts (a mail or social service with OAuth 1 or OAuth 2 credentials), kept for the app.
//!
//! This is the pure model: account types with their access grant, accounts with a typed credential, and a store that
//! saves, looks up and removes them with the official error cases. A credential round-trips through [`Credential::encode`]
//! and [`Credential::decode`], so an app can keep it as an opaque secret in [`crate::keychain`]. A store opened on a file
//! ([`AccountStore::open`], [`shared_path`]) is the one list that every app on the machine reads and writes: changes are
//! written atomically, another handle's changes show after [`AccountStore::reload`], and observers hear each change
//! ([`AccountStore::observe_did_change`]). Credentials stay out of the file. The user's access prompt is still a gap (see
//! `docs/parity/accounts.md`). The framework is deprecated by its vendor, so this exists for apps that still read it.

use crate::error::ErrorCode;
use crate::keychain::{self, Reply};
use crate::Ctx;
use bifrost_core::fsutil;
use bifrost_ui::account_store::{self, AccountEntry, Contents, TypeEntry};
use std::ffi::OsString;
use std::fmt;
use std::io;
use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

/// The credential an account authenticates with (`ACAccountCredential`): OAuth 1 (`init(oAuthToken:tokenSecret:)`)
/// or OAuth 2 (`init(oAuth2Token:refreshToken:expiryDate:)`).
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Credential {
    OAuth { token: String, token_secret: String },
    OAuth2 { token: String, refresh_token: String, expiry: SystemTime },
}

const TAG_OAUTH: u8 = 1;
const TAG_OAUTH2: u8 = 2;

impl Credential {
    /// `init(oAuthToken:tokenSecret:)`
    pub fn oauth(token: &str, token_secret: &str) -> Credential {
        Credential::OAuth { token: token.into(), token_secret: token_secret.into() }
    }

    /// `init(oAuth2Token:refreshToken:expiryDate:)`
    pub fn oauth2(token: &str, refresh_token: &str, expiry: SystemTime) -> Credential {
        Credential::OAuth2 { token: token.into(), refresh_token: refresh_token.into(), expiry }
    }

    /// `oauthToken`: the token the credential authenticates with.
    pub fn oauth_token(&self) -> &str {
        match self {
            Credential::OAuth { token, .. } | Credential::OAuth2 { token, .. } => token,
        }
    }

    /// The OAuth 2 refresh token; `None` for an OAuth 1 credential.
    pub fn refresh_token(&self) -> Option<&str> {
        match self {
            Credential::OAuth { .. } => None,
            Credential::OAuth2 { refresh_token, .. } => Some(refresh_token),
        }
    }

    /// The OAuth 2 expiry date; `None` for an OAuth 1 credential, which does not expire.
    pub fn expiry_date(&self) -> Option<SystemTime> {
        match self {
            Credential::OAuth { .. } => None,
            Credential::OAuth2 { expiry, .. } => Some(*expiry),
        }
    }

    /// True once `now` has reached the OAuth 2 expiry date. An OAuth 1 credential never expires.
    pub fn is_expired(&self, now: SystemTime) -> bool {
        self.expiry_date().is_some_and(|expiry| now >= expiry)
    }

    /// Whether an OAuth 2 credential expires within `margin` of `now`, or has expired, so the app renews it before a
    /// request fails. An OAuth 1 credential never needs renewal.
    pub fn needs_renewal(&self, now: SystemTime, margin: Duration) -> bool {
        match self.expiry_date() {
            Some(expiry) => now.checked_add(margin).map_or(true, |horizon| horizon >= expiry),
            None => false,
        }
    }

    /// The credential as bytes for a keychain item: a tag, then each string as a u32 big-endian length and its bytes,
    /// then (OAuth 2) the expiry as whole seconds since the Unix epoch in a u64 big-endian. Sub-second precision is lost.
    pub fn encode(&self) -> Vec<u8> {
        let mut out = Vec::new();
        match self {
            Credential::OAuth { token, token_secret } => {
                out.push(TAG_OAUTH);
                push_str(&mut out, token);
                push_str(&mut out, token_secret);
            }
            Credential::OAuth2 { token, refresh_token, expiry } => {
                out.push(TAG_OAUTH2);
                push_str(&mut out, token);
                push_str(&mut out, refresh_token);
                let secs = expiry.duration_since(UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0);
                out.extend_from_slice(&secs.to_be_bytes());
            }
        }
        out
    }

    /// Reads a credential written by [`Credential::encode`]. Any other or truncated input is `FailedSerializingAccountInfo`.
    pub fn decode(bytes: &[u8]) -> Result<Credential, AccountError> {
        let mut rest = bytes;
        let tag = take(&mut rest, 1)?[0];
        let credential = match tag {
            TAG_OAUTH => {
                let token = take_str(&mut rest)?;
                let token_secret = take_str(&mut rest)?;
                Credential::OAuth { token, token_secret }
            }
            TAG_OAUTH2 => {
                let token = take_str(&mut rest)?;
                let refresh_token = take_str(&mut rest)?;
                let secs = u64::from_be_bytes(take(&mut rest, 8)?.try_into().map_err(|_| AccountError::FailedSerializingAccountInfo)?);
                Credential::OAuth2 { token, refresh_token, expiry: UNIX_EPOCH + Duration::from_secs(secs) }
            }
            _ => return Err(AccountError::FailedSerializingAccountInfo),
        };
        if rest.is_empty() { Ok(credential) } else { Err(AccountError::FailedSerializingAccountInfo) }
    }
}

fn push_str(out: &mut Vec<u8>, s: &str) {
    out.extend_from_slice(&(s.len() as u32).to_be_bytes());
    out.extend_from_slice(s.as_bytes());
}

fn take<'a>(rest: &mut &'a [u8], n: usize) -> Result<&'a [u8], AccountError> {
    if rest.len() < n {
        return Err(AccountError::FailedSerializingAccountInfo);
    }
    let (head, tail) = rest.split_at(n);
    *rest = tail;
    Ok(head)
}

fn take_str(rest: &mut &[u8]) -> Result<String, AccountError> {
    let len = u32::from_be_bytes(take(rest, 4)?.try_into().map_err(|_| AccountError::FailedSerializingAccountInfo)?) as usize;
    String::from_utf8(take(rest, len)?.to_vec()).map_err(|_| AccountError::FailedSerializingAccountInfo)
}

/// The error cases of the official error code list (`ACErrorCode`) that this model can produce.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AccountError {
    /// The account has no username.
    MissingRequiredProperty,
    /// The account names an account type the store does not have.
    AccountTypeInvalid,
    /// The account type has not been granted to the app (`accessGranted` is false).
    PermissionDenied,
    /// The account has an identifier the store does not hold, so it was removed or never saved.
    UpdatingNonexistentAccount,
    /// No account has the identifier.
    NotFound,
    /// The account has no credential.
    CredentialNotFound,
    /// A credential could not be encoded or decoded.
    FailedSerializingAccountInfo,
}

impl fmt::Display for AccountError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(ErrorCode::description(self))
    }
}

/// The domain of [`AccountError`] in an [`AppError`](crate::error::AppError).
pub const ACCOUNT_DOMAIN: &str = "bifrost.accounts";

impl ErrorCode for AccountError {
    const DOMAIN: &'static str = ACCOUNT_DOMAIN;

    /// Codes 1 to 7 in declaration order. The official list has more cases; the store produces only these.
    fn code(&self) -> i64 {
        match self {
            AccountError::MissingRequiredProperty => 1,
            AccountError::AccountTypeInvalid => 2,
            AccountError::PermissionDenied => 3,
            AccountError::UpdatingNonexistentAccount => 4,
            AccountError::NotFound => 5,
            AccountError::CredentialNotFound => 6,
            AccountError::FailedSerializingAccountInfo => 7,
        }
    }

    fn name(&self) -> &'static str {
        match self {
            AccountError::MissingRequiredProperty => "MissingRequiredProperty",
            AccountError::AccountTypeInvalid => "AccountTypeInvalid",
            AccountError::PermissionDenied => "PermissionDenied",
            AccountError::UpdatingNonexistentAccount => "UpdatingNonexistentAccount",
            AccountError::NotFound => "NotFound",
            AccountError::CredentialNotFound => "CredentialNotFound",
            AccountError::FailedSerializingAccountInfo => "FailedSerializingAccountInfo",
        }
    }

    fn description(&self) -> &'static str {
        match self {
            AccountError::MissingRequiredProperty => "the account has no username",
            AccountError::AccountTypeInvalid => "the account type is not registered",
            AccountError::PermissionDenied => "the app may not use this account type",
            AccountError::UpdatingNonexistentAccount => "the account to update is not in the store",
            AccountError::NotFound => "no account has this identifier",
            AccountError::CredentialNotFound => "the account has no credential",
            AccountError::FailedSerializingAccountInfo => "the credential could not be read or written",
        }
    }
}

/// The account types the system defines: identifier and description. They match the kinds of the Internet Accounts
/// page (mail, CalDAV calendars and reminders, CardDAV contacts) and the BifrOSt account, so an app can name them.
pub const BUILTIN_TYPES: [(&str, &str); 4] = [
    ("bifrost.mail", "Mail"),
    ("bifrost.caldav", "Calendars and Reminders"),
    ("bifrost.carddav", "Contacts"),
    ("bifrost.cloud", "BifrOSt Account"),
];

/// An account type (`ACAccountType`): all accounts of one kind, and whether the app may use them.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AccountType {
    pub identifier: String,
    pub description: String,
    /// `accessGranted`: set by the user's answer to the access request.
    pub access_granted: bool,
}

/// One of the user's accounts (`ACAccount`). The identifier is empty until the store saves the account.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Account {
    pub identifier: String,
    pub account_type: String,
    pub username: String,
    pub user_full_name: String,
    pub account_description: String,
    pub credential: Option<Credential>,
}

impl Account {
    /// A new, unsaved account of the given type.
    pub fn new(account_type: &str, username: &str) -> Account {
        Account {
            identifier: String::new(),
            account_type: account_type.into(),
            username: username.into(),
            user_full_name: String::new(),
            account_description: String::new(),
            credential: None,
        }
    }
}

/// The account store (`ACAccountStore`): the account types, and the accounts saved in them.
///
/// A store opened on a file ([`AccountStore::open`]) keeps that file current. Every save, removal, grant and
/// registration runs against the file's contents as they are at that moment, so two handles on one path never lose
/// each other's writes. A handle sees another's changes after [`AccountStore::reload`]. Credentials are never written
/// to the file: a handle keeps the credentials it already holds in memory, and after a reopen the app sets them again
/// from its keychain item.
#[derive(Default)]
pub struct AccountStore {
    data: Data,
    path: Option<PathBuf>,
    generation: u64,
    observers: Vec<(u64, Box<dyn FnMut()>)>,
    next_observer: u64,
}

/// The state of a store: what the file holds, and the credentials kept in memory.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
struct Data {
    types: Vec<AccountType>,
    accounts: Vec<Account>,
    next_id: u64,
}

impl Data {
    fn account_type(&self, identifier: &str) -> Option<&AccountType> {
        self.types.iter().find(|t| t.identifier == identifier)
    }

    fn set_access(&mut self, identifier: &str, granted: bool) -> Result<(), AccountError> {
        let t = self.types.iter_mut().find(|t| t.identifier == identifier).ok_or(AccountError::AccountTypeInvalid)?;
        t.access_granted = granted;
        Ok(())
    }
}

impl AccountStore {
    /// An empty store that lives only in this process.
    pub fn new() -> AccountStore {
        AccountStore::default()
    }

    /// Opens the store kept in `path`. A missing file is an empty store; a file that cannot be read or parsed is an error.
    pub fn open(path: impl AsRef<Path>) -> io::Result<AccountStore> {
        let path = path.as_ref().to_path_buf();
        let data = read_data(&path)?;
        Ok(AccountStore { data, path: Some(path), ..AccountStore::default() })
    }

    /// The file this store keeps current; `None` for a store made with [`AccountStore::new`].
    pub fn path(&self) -> Option<&Path> {
        self.path.as_deref()
    }

    /// Re-reads the file so that another handle's changes become visible. Returns whether the state changed, and
    /// observers are notified when it did. A store without a file has nothing to read and returns `Ok(false)`.
    pub fn reload(&mut self) -> io::Result<bool> {
        let path = match self.path.clone() {
            Some(path) => path,
            None => return Ok(false),
        };
        let next = self.load_disk(&path)?;
        Ok(self.replace(next))
    }

    /// `ACAccountStoreDidChange`: calls `observer` after each change to the accounts, the types or their grants.
    /// Returns a token for [`AccountStore::unobserve`].
    pub fn observe_did_change(&mut self, observer: impl FnMut() + 'static) -> u64 {
        self.next_observer += 1;
        let boxed: Box<dyn FnMut()> = Box::new(observer);
        self.observers.push((self.next_observer, boxed));
        self.next_observer
    }

    /// Removes the observer with this token. Returns false when there is no such observer.
    pub fn unobserve(&mut self, token: u64) -> bool {
        let before = self.observers.len();
        self.observers.retain(|(t, _)| *t != token);
        self.observers.len() != before
    }

    /// The number of changes this handle has applied: each committed write and each reload that found new state.
    pub fn change_count(&self) -> u64 {
        self.generation
    }

    /// `accountType(withAccountTypeIdentifier:)`
    pub fn account_type(&self, identifier: &str) -> Option<&AccountType> {
        self.data.account_type(identifier)
    }

    /// Registers an account type, not yet granted. Re-registering an identifier replaces its description and keeps its grant.
    pub fn register_type(&mut self, identifier: &str, description: &str) -> Result<(), AccountError> {
        self.mutate(|d| {
            match d.types.iter_mut().find(|t| t.identifier == identifier) {
                Some(t) => t.description = description.into(),
                None => d.types.push(AccountType { identifier: identifier.into(), description: description.into(), access_granted: false }),
            }
            Ok(())
        })
    }

    /// Registers the account types the system defines ([`BUILTIN_TYPES`]) that the store does not hold yet, and returns
    /// how many it added. Existing types keep their description and grant, so calling it at every start is harmless.
    pub fn register_builtin_types(&mut self) -> Result<usize, AccountError> {
        self.mutate(|d| {
            let mut added = 0;
            for (identifier, description) in BUILTIN_TYPES {
                if d.account_type(identifier).is_none() {
                    d.types.push(AccountType { identifier: identifier.into(), description: description.into(), access_granted: false });
                    added += 1;
                }
            }
            Ok(added)
        })
    }

    /// Records the user's answer to the access request for a type (`accessGranted`). An unknown type is `AccountTypeInvalid`.
    pub fn set_access(&mut self, identifier: &str, granted: bool) -> Result<(), AccountError> {
        self.mutate(|d| d.set_access(identifier, granted))
    }

    /// `accounts`: every saved account, as of the last load, change or reload.
    pub fn accounts(&self) -> &[Account] {
        &self.data.accounts
    }

    /// `account(withIdentifier:)`
    pub fn account(&self, identifier: &str) -> Option<&Account> {
        self.data.accounts.iter().find(|a| a.identifier == identifier)
    }

    /// `accounts(with:)`: every saved account of one type.
    pub fn accounts_with_type(&self, account_type: &str) -> Vec<&Account> {
        self.data.accounts.iter().filter(|a| a.account_type == account_type).collect()
    }

    /// `saveAccount`: adds an account with an empty identifier (the store assigns one and returns it), or replaces the
    /// saved account with the same identifier. Checks, in order: the type is registered, the app may use the type, the
    /// username is set, and an identifier that is set names an account the store holds.
    pub fn save(&mut self, account: Account) -> Result<String, AccountError> {
        self.mutate(move |d| {
            let mut account = account;
            let granted = d.account_type(&account.account_type).ok_or(AccountError::AccountTypeInvalid)?.access_granted;
            if !granted {
                return Err(AccountError::PermissionDenied);
            }
            if account.username.is_empty() {
                return Err(AccountError::MissingRequiredProperty);
            }
            if account.identifier.is_empty() {
                d.next_id += 1;
                account.identifier = format!("account-{}", d.next_id);
                d.accounts.push(account.clone());
            } else {
                let slot = d.accounts.iter_mut().find(|a| a.identifier == account.identifier).ok_or(AccountError::UpdatingNonexistentAccount)?;
                *slot = account.clone();
            }
            Ok(account.identifier)
        })
    }

    /// `removeAccount`: takes the account out of the store and returns it. `NotFound` when no account has the identifier.
    pub fn remove(&mut self, identifier: &str) -> Result<Account, AccountError> {
        self.mutate(|d| {
            let i = d.accounts.iter().position(|a| a.identifier == identifier).ok_or(AccountError::NotFound)?;
            Ok(d.accounts.remove(i))
        })
    }

    /// The credential of a saved account. `NotFound` for an unknown identifier, `CredentialNotFound` when it has none.
    pub fn credential(&self, identifier: &str) -> Result<&Credential, AccountError> {
        let account = self.account(identifier).ok_or(AccountError::NotFound)?;
        account.credential.as_ref().ok_or(AccountError::CredentialNotFound)
    }

    /// Whether a saved account's credential needs renewal within `margin` of `now` (see [`Credential::needs_renewal`]).
    /// Checks the account (`NotFound`) and its credential (`CredentialNotFound`) first.
    pub fn renewal_due(&self, identifier: &str, now: SystemTime, margin: Duration) -> Result<bool, AccountError> {
        Ok(self.credential(identifier)?.needs_renewal(now, margin))
    }

    /// `renewCredentials(for:completion:)`: applies the service's answer to a saved account's credential. `Renewed`
    /// replaces the credential. `Rejected` means the user revoked the app's access, so the account type's access is
    /// withdrawn and the old credential is kept. `Failed` changes nothing. Checks, in order: the account exists
    /// (`NotFound`), it has a credential (`CredentialNotFound`), and its type is still granted (`PermissionDenied`).
    pub fn renew_credentials(&mut self, identifier: &str, outcome: RenewOutcome) -> Result<RenewResult, AccountError> {
        self.mutate(move |d| {
            let index = d.accounts.iter().position(|a| a.identifier == identifier).ok_or(AccountError::NotFound)?;
            if d.accounts[index].credential.is_none() {
                return Err(AccountError::CredentialNotFound);
            }
            let account_type = d.accounts[index].account_type.clone();
            if !d.account_type(&account_type).is_some_and(|t| t.access_granted) {
                return Err(AccountError::PermissionDenied);
            }
            match outcome {
                RenewOutcome::Renewed(credential) => {
                    d.accounts[index].credential = Some(credential);
                    Ok(RenewResult::Renewed)
                }
                RenewOutcome::Rejected => {
                    d.set_access(&account_type, false)?;
                    Ok(RenewResult::Rejected)
                }
                RenewOutcome::Failed => Ok(RenewResult::Failed),
            }
        })
    }

    /// Applies one change to the current state, writes the result to the file and notifies observers. A refused change
    /// writes nothing and notifies nobody. A file that cannot be read or written is `FailedSerializingAccountInfo`.
    fn mutate<T>(&mut self, change: impl FnOnce(&mut Data) -> Result<T, AccountError>) -> Result<T, AccountError> {
        let path = self.path.clone();
        let mut next = match &path {
            Some(path) => self.load_disk(path).map_err(|_| AccountError::FailedSerializingAccountInfo)?,
            None => self.data.clone(),
        };
        let out = change(&mut next)?;
        if let Some(path) = &path {
            fsutil::write_atomic(path, encode(&next)).map_err(|_| AccountError::FailedSerializingAccountInfo)?;
        }
        self.replace(next);
        Ok(out)
    }

    /// Makes `next` the state of this handle. Returns whether it differed; a difference counts as a change and notifies.
    fn replace(&mut self, next: Data) -> bool {
        let changed = next != self.data;
        self.data = next;
        if changed {
            self.generation += 1;
            for (_, observer) in self.observers.iter_mut() {
                observer();
            }
        }
        changed
    }

    /// The file's current state, with the credentials this handle already holds carried over (they are not in the file).
    fn load_disk(&self, path: &Path) -> io::Result<Data> {
        let mut data = read_data(path)?;
        for account in &mut data.accounts {
            account.credential = self.data.accounts.iter().find(|a| a.identifier == account.identifier).and_then(|a| a.credential.clone());
        }
        Ok(data)
    }
}

impl fmt::Debug for AccountStore {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("AccountStore").field("types", &self.data.types).field("accounts", &self.data.accounts).field("path", &self.path).finish()
    }
}

/// The keychain service every saved account credential is stored under.
pub const CREDENTIAL_SERVICE: &str = "bifrost.accounts";

/// The keychain account name of one saved account's credential: `credential:<identifier>`.
pub fn credential_account(identifier: &str) -> String {
    format!("credential:{identifier}")
}

/// Replaces the keychain secret of an account's credential with `credential`. The reply is `ItemNotFound` when the
/// account has no item yet; the app then calls [`save_credential_add`] with the same arguments.
pub fn save_credential(ctx: &Ctx, identifier: &str, credential: &Credential) -> u32 {
    keychain::update(ctx, CREDENTIAL_SERVICE, &credential_account(identifier), &credential.encode())
}

/// Adds the keychain item of an account's credential. The reply is `DuplicateItem` when the item exists.
pub fn save_credential_add(ctx: &Ctx, identifier: &str, credential: &Credential) -> u32 {
    keychain::add(ctx, CREDENTIAL_SERVICE, &credential_account(identifier), &credential.encode())
}

/// Reads an account's credential back from the keychain. Its reply goes through [`credential_from_reply`].
pub fn read_credential(ctx: &Ctx, identifier: &str) -> u32 {
    keychain::get(ctx, CREDENTIAL_SERVICE, &credential_account(identifier))
}

/// The credential a keychain reply to [`read_credential`] carries. A reply that is not a success, or whose secret is
/// not a credential, is `CredentialNotFound`, so the app signs the user in again.
pub fn credential_from_reply(reply: &Reply) -> Result<Credential, AccountError> {
    match &reply.result {
        Ok(bytes) => Credential::decode(bytes).map_err(|_| AccountError::CredentialNotFound),
        Err(_) => Err(AccountError::CredentialNotFound),
    }
}

/// The file name of the store the apps share, inside the `bifrost` directory of the user's data directory.
pub const SHARED_FILE: &str = account_store::FILE_NAME;

/// The path of the store the apps share (`$XDG_DATA_HOME/bifrost/accounts.ini`, or under `$HOME/.local/share`).
pub fn shared_path() -> Option<PathBuf> {
    shared_path_from(std::env::var_os("XDG_DATA_HOME"), std::env::var_os("HOME"))
}

fn shared_path_from(xdg_data_home: Option<OsString>, home: Option<OsString>) -> Option<PathBuf> {
    account_store::path_from(xdg_data_home, home)
}

fn read_data(path: &Path) -> io::Result<Data> {
    account_store::read(path).map(from_contents)
}

/// The file's text for the state; the format lives in `bifrost_ui::account_store`.
fn encode(data: &Data) -> String {
    account_store::encode(&to_contents(data))
}

fn to_contents(data: &Data) -> Contents {
    Contents {
        next_id: data.next_id,
        types: data
            .types
            .iter()
            .map(|t| TypeEntry { identifier: t.identifier.clone(), description: t.description.clone(), access_granted: t.access_granted })
            .collect(),
        // a credential never reaches the file
        accounts: data
            .accounts
            .iter()
            .map(|a| AccountEntry {
                identifier: a.identifier.clone(),
                account_type: a.account_type.clone(),
                username: a.username.clone(),
                user_full_name: a.user_full_name.clone(),
                account_description: a.account_description.clone(),
            })
            .collect(),
    }
}

fn from_contents(c: Contents) -> Data {
    Data {
        types: c.types.into_iter().map(|t| AccountType { identifier: t.identifier, description: t.description, access_granted: t.access_granted }).collect(),
        accounts: c
            .accounts
            .into_iter()
            .map(|a| Account {
                identifier: a.identifier,
                account_type: a.account_type,
                username: a.username,
                user_full_name: a.user_full_name,
                account_description: a.account_description,
                credential: None,
            })
            .collect(),
        next_id: c.next_id,
    }
}

/// The answer to a credential renewal (`ACAccountCredentialRenewResult`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RenewResult {
    /// The credentials were renewed and are now the account's.
    Renewed,
    /// Renewal failed because the user revoked the app's access to the account.
    Rejected,
    /// The renewal did not complete for a reason other than the user's refusal.
    Failed,
}

/// What the service answered to a renewal request, as the input to [`AccountStore::renew_credentials`].
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum RenewOutcome {
    Renewed(Credential),
    Rejected,
    Failed,
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::Cell;
    use std::fs;
    use std::rc::Rc;

    fn at(secs: u64) -> SystemTime {
        UNIX_EPOCH + Duration::from_secs(secs)
    }

    fn store_with_type() -> AccountStore {
        let mut store = AccountStore::new();
        store.register_type("mail.example", "Mail").unwrap();
        store
    }

    #[test]
    fn a_keychain_reply_carries_the_credential_back() {
        let c = Credential::oauth2("tok", "ref", at(1_000));
        let ok = Reply { request: 1, result: Ok(c.encode()) };
        assert_eq!(credential_from_reply(&ok), Ok(c));
        let missing = Reply { request: 2, result: Err(keychain::Error::ItemNotFound) };
        assert_eq!(credential_from_reply(&missing), Err(AccountError::CredentialNotFound));
        let junk = Reply { request: 3, result: Ok(b"not a credential".to_vec()) };
        assert_eq!(credential_from_reply(&junk), Err(AccountError::CredentialNotFound));
    }

    #[test]
    fn every_internet_accounts_kind_is_a_builtin_type() {
        for kind in bifrost_ui::internet_accounts::Kind::ALL {
            assert!(BUILTIN_TYPES.iter().any(|(id, _)| *id == kind.account_type()), "{kind:?} has no builtin type");
        }
    }

    #[test]
    fn credential_items_are_named_per_account() {
        assert_eq!(CREDENTIAL_SERVICE, "bifrost.accounts");
        assert_eq!(credential_account("mail-1"), "credential:mail-1");
        assert_ne!(credential_account("a"), credential_account("b"));
    }

    #[test]
    fn builtin_types_are_registered_once_and_keep_their_grant() {
        let mut store = AccountStore::new();
        assert_eq!(store.register_builtin_types(), Ok(BUILTIN_TYPES.len()));
        assert_eq!(store.register_builtin_types(), Ok(0), "a second start adds nothing");
        assert_eq!(store.account_type("bifrost.mail").map(|t| t.description.as_str()), Some("Mail"));
        assert!(store.account_type("bifrost.cloud").is_some_and(|t| !t.access_granted), "built-in types start ungranted");
        store.set_access("bifrost.mail", true).unwrap();
        store.register_builtin_types().unwrap();
        assert!(store.account_type("bifrost.mail").is_some_and(|t| t.access_granted));
    }

    #[test]
    fn builtin_types_survive_a_reopen() {
        let path = scratch("builtin-types");
        let mut store = AccountStore::open(&path).unwrap();
        store.register_builtin_types().unwrap();
        let reopened = AccountStore::open(&path).unwrap();
        assert!(reopened.account_type("bifrost.carddav").is_some());
        assert_eq!(AccountStore::open(&path).unwrap().register_builtin_types(), Ok(0));
    }

    /// A file path under the temp directory, unique per test name and process, with no file yet.
    fn scratch(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("bifrost-accounts-{}-{}", name, std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        dir.join("accounts.ini")
    }

    /// A store on `path` with the mail type registered and granted.
    fn granted_store(path: &Path) -> AccountStore {
        let mut store = AccountStore::open(path).unwrap();
        store.register_type("mail.example", "Mail").unwrap();
        store.set_access("mail.example", true).unwrap();
        store
    }

    /// Observes the store and returns the count the observer keeps.
    fn counting(store: &mut AccountStore) -> Rc<Cell<u32>> {
        let count = Rc::new(Cell::new(0));
        let seen = Rc::clone(&count);
        store.observe_did_change(move || seen.set(seen.get() + 1));
        count
    }

    #[test]
    fn oauth2_credential_expires_at_its_date_and_oauth1_never_does() {
        let oauth2 = Credential::oauth2("tok", "refresh", at(1_000));
        assert!(!oauth2.is_expired(at(999)));
        assert!(oauth2.is_expired(at(1_000)));
        assert_eq!(oauth2.refresh_token(), Some("refresh"));
        assert_eq!(oauth2.expiry_date(), Some(at(1_000)));

        let oauth1 = Credential::oauth("tok", "secret");
        assert!(!oauth1.is_expired(at(4_000_000_000)));
        assert_eq!(oauth1.refresh_token(), None);
        assert_eq!(oauth1.oauth_token(), "tok");
    }

    #[test]
    fn credentials_round_trip_through_their_bytes() {
        for c in [Credential::oauth("a-token", "a-secret"), Credential::oauth2("t", "r", at(1_700_000_000))] {
            assert_eq!(Credential::decode(&c.encode()), Ok(c));
        }
        let empty = Credential::oauth("", "");
        assert_eq!(Credential::decode(&empty.encode()), Ok(empty));
    }

    #[test]
    fn decode_rejects_unknown_tags_truncation_and_trailing_bytes() {
        let good = Credential::oauth2("t", "r", at(5)).encode();
        assert_eq!(Credential::decode(&[]), Err(AccountError::FailedSerializingAccountInfo));
        assert_eq!(Credential::decode(&[9, 0, 0, 0, 0, 0, 0, 0, 0]), Err(AccountError::FailedSerializingAccountInfo));
        assert_eq!(Credential::decode(&good[..good.len() - 1]), Err(AccountError::FailedSerializingAccountInfo));
        let mut trailing = good.clone();
        trailing.push(0);
        assert_eq!(Credential::decode(&trailing), Err(AccountError::FailedSerializingAccountInfo));
    }

    #[test]
    fn saving_assigns_an_identifier_and_filters_by_type() {
        let mut store = store_with_type();
        store.register_type("chat.example", "Chat").unwrap();
        store.set_access("mail.example", true).unwrap();
        store.set_access("chat.example", true).unwrap();
        let mail = store.save(Account::new("mail.example", "ola@example.is")).unwrap();
        let chat = store.save(Account::new("chat.example", "ola")).unwrap();
        assert_ne!(mail, chat);
        assert_eq!(store.account(&mail).map(|a| a.username.as_str()), Some("ola@example.is"));
        assert_eq!(store.accounts_with_type("mail.example").len(), 1);
        assert_eq!(store.accounts().len(), 2);
    }

    #[test]
    fn save_is_refused_until_the_app_is_granted_the_type() {
        let mut store = store_with_type();
        let account = Account::new("mail.example", "ola");
        assert_eq!(store.save(account.clone()), Err(AccountError::PermissionDenied));
        store.set_access("mail.example", true).unwrap();
        assert!(store.save(account).is_ok());
        store.set_access("mail.example", false).unwrap();
        assert_eq!(store.account_type("mail.example").map(|t| t.access_granted), Some(false));
    }

    #[test]
    fn save_checks_the_type_and_the_required_username() {
        let mut store = store_with_type();
        store.set_access("mail.example", true).unwrap();
        assert_eq!(store.save(Account::new("unknown.example", "ola")), Err(AccountError::AccountTypeInvalid));
        assert_eq!(store.set_access("unknown.example", true), Err(AccountError::AccountTypeInvalid));
        assert_eq!(store.save(Account::new("mail.example", "")), Err(AccountError::MissingRequiredProperty));
        assert!(store.accounts().is_empty());
    }

    #[test]
    fn saving_an_identifier_updates_it_and_an_unknown_one_is_refused() {
        let mut store = store_with_type();
        store.set_access("mail.example", true).unwrap();
        let id = store.save(Account::new("mail.example", "old")).unwrap();
        let mut renamed = store.account(&id).unwrap().clone();
        renamed.username = "new".into();
        renamed.credential = Some(Credential::oauth("t", "s"));
        assert_eq!(store.save(renamed), Ok(id.clone()));
        assert_eq!(store.account(&id).map(|a| a.username.as_str()), Some("new"));
        assert_eq!(store.accounts().len(), 1);

        let mut ghost = Account::new("mail.example", "ghost");
        ghost.identifier = "account-99".into();
        assert_eq!(store.save(ghost), Err(AccountError::UpdatingNonexistentAccount));
    }

    #[test]
    fn remove_returns_the_account_once_and_credentials_are_looked_up() {
        let mut store = store_with_type();
        store.set_access("mail.example", true).unwrap();
        let mut account = Account::new("mail.example", "ola");
        let id = store.save(account.clone()).unwrap();
        assert_eq!(store.credential(&id), Err(AccountError::CredentialNotFound));

        account.identifier = id.clone();
        account.credential = Some(Credential::oauth("t", "s"));
        store.save(account).unwrap();
        assert_eq!(store.credential(&id).map(|c| c.oauth_token()), Ok("t"));
        assert_eq!(store.credential("missing"), Err(AccountError::NotFound));

        assert_eq!(store.remove(&id).map(|a| a.username), Ok("ola".to_string()));
        assert_eq!(store.remove(&id), Err(AccountError::NotFound));
        assert!(store.accounts().is_empty());
    }

    #[test]
    fn registering_a_type_again_keeps_one_entry() {
        let mut store = store_with_type();
        store.register_type("mail.example", "Mail (renamed)").unwrap();
        assert_eq!(store.account_type("mail.example").map(|t| t.description.as_str()), Some("Mail (renamed)"));
        assert_eq!(store.data.types.len(), 1);
    }

    #[test]
    fn save_then_reopen_keeps_types_grants_and_accounts() {
        let path = scratch("round-trip");
        let id = {
            let mut store = granted_store(&path);
            let mut account = Account::new("mail.example", "ola@example.is");
            account.user_full_name = "Ólafur Búi".into();
            account.account_description = "Work".into();
            store.save(account).unwrap()
        };
        let store = AccountStore::open(&path).unwrap();
        let t = store.account_type("mail.example").unwrap();
        assert_eq!(t.description, "Mail");
        assert!(t.access_granted);
        let a = store.account(&id).unwrap();
        assert_eq!(a.account_type, "mail.example");
        assert_eq!(a.username, "ola@example.is");
        assert_eq!(a.user_full_name, "Ólafur Búi");
        assert_eq!(a.account_description, "Work");
        assert_eq!(a.credential, None);
    }

    #[test]
    fn identifiers_are_not_reused_after_a_reopen() {
        let path = scratch("ids");
        let mut store = granted_store(&path);
        let first = store.save(Account::new("mail.example", "a")).unwrap();
        let second = store.save(Account::new("mail.example", "b")).unwrap();
        store.remove(&second).unwrap();
        let mut reopened = AccountStore::open(&path).unwrap();
        let third = reopened.save(Account::new("mail.example", "c")).unwrap();
        assert_eq!(first, "account-1");
        assert_ne!(third, first);
        assert_ne!(third, second);
    }

    #[test]
    fn credentials_stay_out_of_the_plain_file() {
        let path = scratch("secrets");
        let mut store = granted_store(&path);
        let mut account = Account::new("mail.example", "ola");
        account.credential = Some(Credential::oauth2("tok-secret-123", "refresh-secret-456", at(9_000)));
        let id = store.save(account).unwrap();
        let text = fs::read_to_string(&path).unwrap();
        assert!(!text.contains("tok-secret-123"));
        assert!(!text.contains("refresh-secret-456"));
        // the handle that saved it keeps the credential, including across a later save of another account
        store.save(Account::new("mail.example", "other")).unwrap();
        assert_eq!(store.credential(&id).map(|c| c.oauth_token()), Ok("tok-secret-123"));
        let reopened = AccountStore::open(&path).unwrap();
        assert_eq!(reopened.account(&id).map(|a| a.credential.clone()), Some(None));
    }

    #[test]
    fn remove_persists_across_a_reopen() {
        let path = scratch("remove");
        let mut store = granted_store(&path);
        let keep = store.save(Account::new("mail.example", "keep")).unwrap();
        let gone = store.save(Account::new("mail.example", "gone")).unwrap();
        assert_eq!(store.remove(&gone).map(|a| a.username), Ok("gone".to_string()));
        let reopened = AccountStore::open(&path).unwrap();
        assert_eq!(reopened.accounts().len(), 1);
        assert!(reopened.account(&keep).is_some());
        assert!(reopened.account(&gone).is_none());
    }

    #[test]
    fn two_handles_on_one_path_share_one_list() {
        let path = scratch("shared");
        let mut a = granted_store(&path);
        let mut b = AccountStore::open(&path).unwrap();
        assert!(b.account_type("mail.example").unwrap().access_granted);

        let from_a = a.save(Account::new("mail.example", "from-a")).unwrap();
        assert!(b.account(&from_a).is_none());
        assert!(b.reload().unwrap());
        assert!(b.account(&from_a).is_some());
        assert!(!b.reload().unwrap());

        // a saves after b's reload; b's save works from the file, so a's account survives and no identifier repeats
        let second_from_a = a.save(Account::new("mail.example", "second-from-a")).unwrap();
        let from_b = b.save(Account::new("mail.example", "from-b")).unwrap();
        assert_ne!(from_b, from_a);
        assert_ne!(from_b, second_from_a);
        let fresh = AccountStore::open(&path).unwrap();
        assert_eq!(fresh.accounts().len(), 3);
        assert!(fresh.account(&from_a).is_some());
        assert!(fresh.account(&second_from_a).is_some());
        assert!(fresh.account(&from_b).is_some());
    }

    #[test]
    fn a_change_notifies_observers_and_a_refused_one_does_not() {
        let mut store = AccountStore::new();
        store.register_type("mail.example", "Mail").unwrap();
        store.set_access("mail.example", true).unwrap();
        let count = counting(&mut store);
        let id = store.save(Account::new("mail.example", "ola")).unwrap();
        assert_eq!(count.get(), 1);
        assert!(store.save(Account::new("mail.example", "")).is_err());
        assert_eq!(count.get(), 1);
        store.remove(&id).unwrap();
        assert_eq!(count.get(), 2);
        // register, grant, save and remove each changed the state once
        assert_eq!(store.change_count(), 4);
    }

    #[test]
    fn unobserve_stops_the_notifications() {
        let mut store = AccountStore::new();
        store.register_type("mail.example", "Mail").unwrap();
        let count = Rc::new(Cell::new(0));
        let seen = Rc::clone(&count);
        let token = store.observe_did_change(move || seen.set(seen.get() + 1));
        store.set_access("mail.example", true).unwrap();
        assert!(store.unobserve(token));
        assert!(!store.unobserve(token));
        store.set_access("mail.example", false).unwrap();
        assert_eq!(count.get(), 1);
    }

    #[test]
    fn reload_notifies_when_another_handle_changed_the_list() {
        let path = scratch("reload-notify");
        let mut a = granted_store(&path);
        let mut b = AccountStore::open(&path).unwrap();
        let count = counting(&mut b);
        a.save(Account::new("mail.example", "x")).unwrap();
        assert!(b.reload().unwrap());
        assert_eq!(count.get(), 1);
        assert!(!b.reload().unwrap());
        assert_eq!(count.get(), 1);
    }

    #[test]
    fn a_store_without_a_file_has_nothing_to_reload() {
        let mut store = AccountStore::new();
        assert!(store.path().is_none());
        assert!(!store.reload().unwrap());
    }

    #[test]
    fn a_missing_file_is_an_empty_store_and_a_malformed_one_is_an_error() {
        let path = scratch("malformed");
        let empty = AccountStore::open(&path).unwrap();
        assert!(empty.accounts().is_empty());
        assert!(empty.account_type("mail.example").is_none());
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(&path, "[account]\nusername = ola\n").unwrap();
        let err = AccountStore::open(&path).unwrap_err();
        assert_eq!(err.kind(), io::ErrorKind::InvalidData);
        fs::write(&path, "username = ola\n").unwrap();
        assert!(AccountStore::open(&path).is_err());
    }

    #[test]
    fn unknown_sections_and_keys_are_skipped() {
        let path = scratch("forward");
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(&path, "[future]\nthing = 1\n[account]\nid = account-4\ntype = mail.example\nusername = ola\nnickname = o\n").unwrap();
        let store = AccountStore::open(&path).unwrap();
        assert_eq!(store.accounts().len(), 1);
        assert_eq!(store.account("account-4").map(|a| a.username.as_str()), Some("ola"));
    }

    #[test]
    fn values_with_newlines_backslashes_and_edge_spaces_round_trip() {
        let path = scratch("escapes");
        let mut store = granted_store(&path);
        let mut account = Account::new("mail.example", "ola\\n\nx");
        account.user_full_name = "  leading and trailing  ".into();
        account.account_description = "line one\r\nline two \\ end".into();
        let id = store.save(account).unwrap();
        let reopened = AccountStore::open(&path).unwrap();
        let saved = reopened.account(&id).unwrap();
        assert_eq!(saved.username, "ola\\n\nx");
        assert_eq!(saved.user_full_name, "  leading and trailing  ");
        assert_eq!(saved.account_description, "line one\r\nline two \\ end");
    }

    #[test]
    fn the_shared_store_lives_in_the_data_directory_or_under_the_home() {
        assert_eq!(shared_path_from(Some("/data".into()), Some("/home/u".into())), Some(PathBuf::from("/data/bifrost/accounts.ini")));
        assert_eq!(shared_path_from(Some("".into()), Some("/home/u".into())), Some(PathBuf::from("/home/u/.local/share/bifrost/accounts.ini")));
        assert_eq!(shared_path_from(None, None), None);
    }

    /// A store with one granted type and one saved account whose credential is the OAuth 2 token `old`.
    fn renewal_store() -> (AccountStore, String) {
        let mut store = store_with_type();
        store.set_access("mail.example", true).unwrap();
        let mut account = Account::new("mail.example", "ann");
        account.credential = Some(Credential::oauth2("old", "r1", at(1_000)));
        let id = store.save(account).unwrap();
        (store, id)
    }

    #[test]
    fn an_oauth2_credential_needs_renewal_inside_the_margin() {
        let c = Credential::oauth2("t", "r", at(1_000));
        assert!(c.needs_renewal(at(900), Duration::from_secs(100)));
        assert!(!c.needs_renewal(at(899), Duration::from_secs(100)));
        assert!(c.needs_renewal(at(1_500), Duration::ZERO), "an expired credential needs renewal");
        assert!(!Credential::oauth("t", "s").needs_renewal(at(u64::MAX / 4), Duration::from_secs(1)), "OAuth 1 never expires");
    }

    #[test]
    fn renewal_due_checks_the_saved_credential() {
        let (store, id) = renewal_store();
        assert_eq!(store.renewal_due(&id, at(950), Duration::from_secs(100)), Ok(true));
        assert_eq!(store.renewal_due(&id, at(800), Duration::from_secs(100)), Ok(false));
        assert_eq!(store.renewal_due("missing", at(0), Duration::ZERO), Err(AccountError::NotFound));
    }

    #[test]
    fn renewed_credentials_replace_the_saved_ones() {
        let (mut store, id) = renewal_store();
        let fresh = Credential::oauth2("new", "r2", at(9_000));
        assert_eq!(store.renew_credentials(&id, RenewOutcome::Renewed(fresh.clone())), Ok(RenewResult::Renewed));
        assert_eq!(store.credential(&id), Ok(&fresh));
        assert_eq!(store.credential(&id).map(|c| c.oauth_token()), Ok("new"));
    }

    #[test]
    fn a_revoked_renewal_withdraws_access_and_keeps_the_old_credential() {
        let (mut store, id) = renewal_store();
        assert_eq!(store.renew_credentials(&id, RenewOutcome::Rejected), Ok(RenewResult::Rejected));
        assert_eq!(store.account_type("mail.example").map(|t| t.access_granted), Some(false));
        assert_eq!(store.credential(&id).map(|c| c.oauth_token()), Ok("old"));
    }

    #[test]
    fn a_failed_renewal_changes_nothing() {
        let (mut store, id) = renewal_store();
        assert_eq!(store.renew_credentials(&id, RenewOutcome::Failed), Ok(RenewResult::Failed));
        assert_eq!(store.account_type("mail.example").map(|t| t.access_granted), Some(true));
        assert_eq!(store.credential(&id).map(|c| c.oauth_token()), Ok("old"));
    }

    #[test]
    fn renewal_checks_the_account_the_credential_and_the_grant_in_that_order() {
        let (mut store, id) = renewal_store();
        assert_eq!(store.renew_credentials("missing", RenewOutcome::Failed), Err(AccountError::NotFound));
        let bare = store.save(Account::new("mail.example", "bob")).unwrap();
        assert_eq!(store.renew_credentials(&bare, RenewOutcome::Failed), Err(AccountError::CredentialNotFound));
        store.set_access("mail.example", false).unwrap();
        assert_eq!(store.renew_credentials(&id, RenewOutcome::Failed), Err(AccountError::PermissionDenied));
    }
}
