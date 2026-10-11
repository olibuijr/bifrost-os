# Accounts parity

Apple's Accounts framework is deprecated: its landing page tells new apps to use the service provider's SDK. This
note records the pure-Rust model BifrOSt now has for it, so apps that still read the framework's object set have a
typed surface. Matrix row: `docs/parity-matrix/data/accounts.json`. Module: `springboard/sdk/src/accounts.rs`
(`pub mod accounts;` in `springboard/sdk/src/lib.rs`).

## What was built

- `Credential`: an enum with `OAuth` (token and token secret) and `OAuth2` (token, refresh token and expiry date).
  Mirrors `ACAccountCredential`: `oauth`, `oauth2`, `oauth_token`, `refresh_token`, `expiry_date`, `is_expired`.
  OAuth 1 never expires. `encode` and `decode` turn a credential into bytes for an opaque keychain item
  (`springboard/sdk/src/keychain.rs`). Expiry is kept in whole seconds.
- `AccountType`, with `access_granted`. Mirrors `ACAccountType`.
- `Account`, with identifier, account type, username, full name, description and an optional credential. Mirrors `ACAccount`.
- `AccountStore`, in memory. `register_type`, `account_type` (`accountType(withAccountTypeIdentifier:)`),
  `set_access`, `accounts`, `account` (`account(withIdentifier:)`), `accounts_with_type` (`accounts(with:)`),
  `save` (`saveAccount`), `remove` (`removeAccount`), `credential`.
- `save` checks, in order: the type is registered, the app may use it, the username is set, and a non-empty
  identifier names a saved account. An empty identifier is assigned by the store.
- `AccountError`: seven official `ACErrorCode` cases that the model can produce, with a plain `Display` message.

## Official API names mirrored

- Classes: `ACAccountStore`, `ACAccount`, `ACAccountType`, `ACAccountCredential`, `ACErrorCode`.
- Methods: `init(oAuthToken:tokenSecret:)`, `init(oAuth2Token:refreshToken:expiryDate:)`, `oauthToken`,
  `accountType(withAccountTypeIdentifier:)`, `accounts`, `account(withIdentifier:)`, `accounts(with:)`,
  `saveAccount(_:withCompletionHandler:)`, `removeAccount(_:withCompletionHandler:)`, `renewCredentials(for:completion:)`.

Mirrored by behaviour, not by signature: the official calls take completion handlers, and the model is
synchronous. The official names appear only in doc comments.

## REA evidence

Official pages rendered and recorded with `rea-page.sh` (REA passive browser evidence, 2026-10-11):

- Framework root, deprecation and the object list: `ev_d8db1a18c8881336b7b0a660a3db2cd2b3d18f047fe28313778b31bca64b3a88`
- `ACAccountCredential` (deprecated; initializers, `oauthToken`): `ev_e0b4e1daaec739fb2bb7028fe87360d5d0e103facbff592b57cfc83a136e798e`
- `ACAccountStore` (access, get, save, renew, remove topics): `ev_58145270ca9d9c09bd40572a7e5af7fe78a2f598a59519f6ffc612a3d5c5fb8f`
- `ACErrorCode` (error case list): `ev_85e78bc925e1f086f0698f5826a428465d01517914296dd77b9fc8f4c5218b03`

## Remaining gaps

- The store is in memory for one process. A store shared across apps, and persistence through the keychain, are not built.
- No user access prompt. `set_access` is the caller's answer.
- No `renewCredentials` (it needs a network service).
- No SDK API for the local OS account (`springboard/src/bare/account.rs`) or for the BifrOSt cloud account.
- ACErrorCode cases that depend on the system store, keychain or plugins are not modelled. There is no error domain and no numeric raw value.
- Tests: `cargo test -p bifrost-app accounts` covers credential expiry and encoding (round trip and malformed input),
  access grants, save and update rules, remove and credential lookup. Not yet built or run in this change.
