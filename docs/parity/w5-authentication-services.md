# Authentication Services (wave 5)

Two pure-Rust features that mirror the Authentication Services request inputs for a passkey and for a generated
password. Neither touches a key store, an authenticator, the network or a socket. Apps can call them today, but the
output has no caller in the shell yet and has not been run on the VM or Midget.

## What was built

- `springboard/sdk/src/passkey.rs` (`bifrost_app::passkey`): the client data a passkey registration or assertion
  signs. `ClientData::new(Operation, challenge, origin)` checks the challenge (at least 16 bytes) and the origin (an
  `http://` or `https://` scheme and a host with no path, whitespace or quotes). `to_json()` writes `type`,
  `challenge` (base64url, no padding), `origin` and `crossOrigin` in that order. `base64url` reuses the base64
  encoder in `http_auth.rs`.
- `springboard/sdk/src/password_gen.rs` (`bifrost_app::password_gen`): password rules (`Rules`: length and four
  class flags) and the generator. Each enabled class contributes one character, the rest come from the union, and
  a Fisher-Yates shuffle orders them. `generate` draws from `entropy::below` (the kernel pool). `generate_with`
  takes the draw function as an argument so the tests can fix every draw.

## Official API names mirrored

- `ASAuthorizationPlatformPublicKeyCredentialProvider`: `createCredentialAssertionRequest(challenge:)` and
  `createCredentialRegistrationRequest(challenge:name:userID:)`. The challenge input is what `ClientData` carries.
  Source: https://developer.apple.com/documentation/authenticationservices/asauthorizationplatformpublickeycredentialprovider
- `ASGeneratePasswordsRequest`: the generate-password request for a sign-up form (its password rules and service
  identifier). Source: https://developer.apple.com/documentation/authenticationservices/asgeneratepasswordsrequest
- `ASWebAuthenticationSession` is not changed by this wave. The existing `springboard/core/src/auth_session.rs` already
  covers its callback handling.

## REA evidence

- Framework landing page: `ev_d1ac03712140270b435a4620b1564c4e472e365132148f3f77db55bf2e2e50e2`
- `ASGeneratePasswordsRequest`: `ev_8e1576303f594786155d19e9e5a6ab8fecc1d13afa02709b17b28ba25d818945`
- `ASAuthorizationPlatformPublicKeyCredentialProvider`: `ev_f65b5513a4d75fcab2042ba84bff48ec19662b351e09715855fae42eed866215`

The DocC JSON fetch (`docc.py`) returned HTTP 404 for the passkey pages, so their API names come from the rendered
REA pages above.

## Rows closed

| Framework | Feature | Status | BifrOSt path |
|---|---|---|---|
| Authentication Services | Passkeys and FIDO2 (public-key credentials) | partial | springboard/sdk/src/passkey.rs |
| Authentication Services | Password credentials (sign-in form and keychain sharing) | partial | springboard/sdk/src/password_gen.rs (adds generation; the row was already partial) |

## Not built, and why

- Web authentication sessions: `core/src/auth_session.rs` already exists. Nothing new was needed there.
- Passkey registration and assertion: the key pair, the authenticator and the relying-party check need a platform
  authenticator. Those are out of scope for pure Rust with no hardware access.
- Shared credential store, save-password prompt and AutoFill: these need a shared store across apps and a prompt in
  the shell, which are separate features.
- Credential import and export (CXF), one-time passcodes from messages, and single sign-on for the browser: each
  needs a service or an integration outside this change.

## Tests

Each module has `#[cfg(test)] mod tests`, with hand-worked expected values:

- `passkey`: base64url of `f`, `fo` and `[0xfb, 0xff]`; 16 zero bytes give 22 characters; the registration JSON's
  exact string; assertion type and `crossOrigin`; a 15-byte challenge is refused; the origin rules.
- `password_gen`: fixed draws give `93ba`; all-zero draws give `A0!a`; no class and too short are refused; a failing
  source is an entropy error; a generated 20-character password has every class.

Per the wave brief, nothing was built or run (`cargo` was not invoked). `python3 tools/check-names.py` exits 0.
