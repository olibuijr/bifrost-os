# Wave 5, theme 06: single sign-on and credentials (parity)

Theme: "Single sign-on in a web browser app" (19 gap rows, 14 frameworks). This wave closes three rows as partial by
extending the existing credential and account models and adding one module. It does not close the browser SSO row:
its web authentication session model is theme 05 (`springboard/core/src/auth_session.rs`) and is not wired into the
browser yet.

## What was built

- **Credential renewal** (`springboard/sdk/src/accounts.rs`): `AccountStore::renew_credentials(identifier, RenewOutcome)`
  returns `RenewResult` (renewed, rejected, failed). `Renewed` replaces the saved credential. `Rejected` withdraws the
  app's access to the account type and keeps the old credential. `Failed` changes nothing. Checks run in the order
  account exists, credential present, type granted.
- **Challenge answers and credential persistence** (`springboard/sdk/src/http_auth.rs`):
  - `AuthChallenge` (protection space, previous failure count, proposed credential) and `AuthDisposition`
    (use credential, default handling, cancel, reject space). `disposition` answers only a first attempt with a
    credential: the proposed one, then the stored one. After a failure, or with no credential, it asks for default
    handling, so a refused password is not sent again.
  - `CredentialPersistence::Synchronizable` (kept like `Permanent`, for this device only; no other devices are reached).
  - `URLCredentialStorage::to_keychain_bytes` and `from_keychain_bytes`: the permanent and synchronizable entries as one
    versioned byte layout for a keychain item, so they survive a restart once the caller stores them.
- **Passkey client data** (`springboard/sdk/src/passkey_client_data.rs`, new): `ClientData` builds the signed JSON for a
  registration (`webauthn.create`) or sign-in (`webauthn.get`). It checks the origin (https, or http on localhost, with
  no path, query, user info or bad port), encodes the challenge as unpadded base64url, writes the members in
  specification order, and checks the relying party identifier against the origin's host or a parent domain.

## Official API names mirrored

| Module | Official names (DocC) |
|---|---|
| `accounts.rs` | `ACAccountCredentialRenewResult` (`renewed`, `rejected`, `failed`), `ACAccountStore` `renewCredentials(for:completion:)` |
| `http_auth.rs` | `URLSession.AuthChallengeDisposition` (`useCredential`, `performDefaultHandling`, `cancelAuthenticationChallenge`, `rejectProtectionSpace`), `URLAuthenticationChallenge` (`protectionSpace`, `previousFailureCount`, `proposedCredential`), `URLCredential.Persistence` (`none`, `forSession`, `permanent`, `synchronizable`), `URLProtectionSpace` |
| `passkey_client_data.rs` | `ASAuthorizationPublicKeyCredentialRegistrationRequest` (`challenge`, `relyingPartyIdentifier`); the `clientDataJSON` members and `webauthn.create` / `webauthn.get` types come from the W3C WebAuthn specification, not from Apple's pages |

## REA evidence (rendered official pages)

| Page | Evidence id |
|---|---|
| `accounts/acaccountcredentialrenewresult` | `ev_17d4a7c5efa6e52e1b75d2f232762e38b75cc2aa139507775c3f98210c472895` |
| `foundation/urlcredential/persistence-swift.enum` | `ev_f8db0be7f0caea989bb03a942f2641505e5a143487de403bce9424bec94fd98b` |
| `foundation/urlprotectionspace` | `ev_d323934e902c8007f57fe35c023f5bdf0835e3a8495c18054380306821ebbed9` |
| `foundation/urlsession/authchallengedisposition` | `ev_f103f76b3e521cff6e863cfe4af055c6315264ed789b6809114730b6a0b28deb` |
| `foundation/urlauthenticationchallenge` | `ev_2f51b40f7d0d2cca8535bc5bba6142b8547c43e273f1b51a15cc48260b71280f` |
| `authenticationservices/asauthorizationpublickeycredentialregistrationrequest` | `ev_ac50dc8eae4b24aa98bee8331f55150c828acb745f68a313d2c0f8b965f49249` |

## Matrix rows closed

| Framework slug | Feature (as in `data/<slug>.json`) | New status | BifrOSt path |
|---|---|---|---|
| accounts | Credential renewal (renewCredentials) | partial | `springboard/sdk/src/accounts.rs` (`AccountStore::renew_credentials`) |
| foundation-networking-lowlevel | URL Loading: cookies, credentials, authentication challenges | partial | `springboard/sdk/src/http_auth.rs` (`AuthChallenge`, `URLCredentialStorage::to_keychain_bytes`); cookies stay in `springboard/sdk/src/http_cookies.rs` |
| authentication-services | Passkeys and FIDO2 (public-key credentials) | partial | `springboard/sdk/src/passkey_client_data.rs` (`ClientData`) |

## Not closed, and why

- **Single sign-on in a web browser app**: theme 05's `WebSession` model exists but is not wired into the browser.
- **Authorization requests (controller, results)**, **AutoFill credential provider**, **Credential import and export**,
  **Authentication sessions (shared Safari login)**: no pure model added in this wave.
- **Passkeys**: only the client data is modelled. There is no authenticator, no key generation and no signing, so no
  passkey can be created or used. The row is partial, not parity.
- **Cookies**: `http_cookies.rs` (wave 3) is unchanged. The URL system still exposes no cookie API to apps.
- **Universal links** (`app_links.rs`) and **Security Foundation rights** were not changed.

## Verification

No builds or tests were run (task rule). The code and the expected values in the new tests were checked by hand:
the RFC 4648 and base64url vectors (`Zm8`, `-_8`, `_w`), the keychain byte vector
`[1,0,0,0,1,0,0,0,1,97,1,187,0,0,0,0,1,117,0,0,0,1,112,2]` (port 443 is `0x01BB`), and the origin and relying party
rules. `python3 tools/check-names.py` reports no hits.
