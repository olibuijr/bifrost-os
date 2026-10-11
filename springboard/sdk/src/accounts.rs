//! Accounts (`ACAccountStore`, `ACAccount`, `ACAccountType`, `ACAccountCredential`, `ACErrorCode`): the user's
//! external accounts (a mail or social service with OAuth 1 or OAuth 2 credentials), kept for the app.
//!
//! This is the pure model: account types with their access grant, accounts with a typed credential, and a store that
//! saves, looks up and removes them with the official error cases. A credential round-trips through [`Credential::encode`]
//! and [`Credential::decode`], so an app can keep it as an opaque secret in [`crate::keychain`]. The store itself lives in
//! memory for the app's process; a store shared between apps and the user's access prompt are still gaps (see
//! `docs/parity/accounts.md`). The framework is deprecated by its vendor, so this exists for apps that still read it.

use crate::error::ErrorCode;
use std::fmt;
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
#[derive(Debug, Default)]
pub struct AccountStore {
    types: Vec<AccountType>,
    accounts: Vec<Account>,
    next_id: u64,
}

impl AccountStore {
    pub fn new() -> AccountStore {
        AccountStore::default()
    }

    /// Registers an account type, not yet granted. Re-registering an identifier replaces its description.
    pub fn register_type(&mut self, identifier: &str, description: &str) {
        match self.types.iter_mut().find(|t| t.identifier == identifier) {
            Some(t) => t.description = description.into(),
            None => self.types.push(AccountType { identifier: identifier.into(), description: description.into(), access_granted: false }),
        }
    }

    /// `accountType(withAccountTypeIdentifier:)`
    pub fn account_type(&self, identifier: &str) -> Option<&AccountType> {
        self.types.iter().find(|t| t.identifier == identifier)
    }

    /// Records the user's answer to the access request for a type (`accessGranted`). An unknown type is `AccountTypeInvalid`.
    pub fn set_access(&mut self, identifier: &str, granted: bool) -> Result<(), AccountError> {
        let t = self.types.iter_mut().find(|t| t.identifier == identifier).ok_or(AccountError::AccountTypeInvalid)?;
        t.access_granted = granted;
        Ok(())
    }

    /// `accounts`: every saved account.
    pub fn accounts(&self) -> &[Account] {
        &self.accounts
    }

    /// `account(withIdentifier:)`
    pub fn account(&self, identifier: &str) -> Option<&Account> {
        self.accounts.iter().find(|a| a.identifier == identifier)
    }

    /// `accounts(with:)`: every saved account of one type.
    pub fn accounts_with_type(&self, account_type: &str) -> Vec<&Account> {
        self.accounts.iter().filter(|a| a.account_type == account_type).collect()
    }

    /// `saveAccount`: adds an account with an empty identifier (the store assigns one and returns it), or replaces the
    /// saved account with the same identifier. Checks, in order: the type is registered, the app may use the type, the
    /// username is set, and an identifier that is set names an account the store holds.
    pub fn save(&mut self, mut account: Account) -> Result<String, AccountError> {
        let granted = self.account_type(&account.account_type).ok_or(AccountError::AccountTypeInvalid)?.access_granted;
        if !granted {
            return Err(AccountError::PermissionDenied);
        }
        if account.username.is_empty() {
            return Err(AccountError::MissingRequiredProperty);
        }
        if account.identifier.is_empty() {
            self.next_id += 1;
            account.identifier = format!("account-{}", self.next_id);
            self.accounts.push(account.clone());
        } else {
            let slot = self.accounts.iter_mut().find(|a| a.identifier == account.identifier).ok_or(AccountError::UpdatingNonexistentAccount)?;
            *slot = account.clone();
        }
        Ok(account.identifier)
    }

    /// `removeAccount`: takes the account out of the store and returns it. `NotFound` when no account has the identifier.
    pub fn remove(&mut self, identifier: &str) -> Result<Account, AccountError> {
        let i = self.accounts.iter().position(|a| a.identifier == identifier).ok_or(AccountError::NotFound)?;
        Ok(self.accounts.remove(i))
    }

    /// The credential of a saved account. `NotFound` for an unknown identifier, `CredentialNotFound` when it has none.
    pub fn credential(&self, identifier: &str) -> Result<&Credential, AccountError> {
        let account = self.account(identifier).ok_or(AccountError::NotFound)?;
        account.credential.as_ref().ok_or(AccountError::CredentialNotFound)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn at(secs: u64) -> SystemTime {
        UNIX_EPOCH + Duration::from_secs(secs)
    }

    fn store_with_type() -> AccountStore {
        let mut store = AccountStore::new();
        store.register_type("mail.example", "Mail");
        store
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
        store.register_type("chat.example", "Chat");
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
        store.register_type("mail.example", "Mail (renamed)");
        assert_eq!(store.account_type("mail.example").map(|t| t.description.as_str()), Some("Mail (renamed)"));
        assert_eq!(store.types.len(), 1);
    }
}
