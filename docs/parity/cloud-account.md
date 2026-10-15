# BifrOSt Cloud sign-in for apps

Matrix row: `docs/parity-matrix/data/accounts.json`, feature "BifrOSt account sign-in (cloud)". Module:
`springboard/sdk/src/cloud_account.rs` (`bifrost_app::cloud_account`). Tests: `springboard/sdk/tests/parity_sdk-rest-15.rs`.

## What it mirrors

The BifrOSt Cloud HTTP API that the cloud service exposes for accounts (its own repository). Apple's equivalent is
the Apple Account sign-in, which is a service of the reference vendor; this module is the BifrOSt side of that
service for apps, not a copy of the vendor's sign-in sheet.

- `normalize_email` applies the service's address rules (trim, lowercase, at most 254 bytes, one `@`, a dotted domain,
  no inner whitespace).
- `SignIn::new` checks the email and requires a password. Its `Debug` output hides the password.
- `SignIn::request(base)` builds `POST {base}/signin` with a JSON body. The base must be an `https` URL.
- `parse_sign_in` reads a `200` answer into a `SignedIn` (the account and the `bc_session` token). Other statuses map
  to `BadCredentials` (401), `RateLimited` (429), `InvalidEmail` (400), or `Rejected` with the status and error code.
- `sign_out_request` and `parse_sign_out` end the session with `POST {base}/signout` and its `204` answer.
- `sign_in` and `session_send` take the transport: a closure in tests, the URL session in the app.

## What is tested headless

Canned transports and literal answers cover the email rules, the request URL, headers and body, the password redaction,
the success and failure mappings, the missing cookie and missing field cases, the answer size limit, and the
sign-out cookie. No socket is opened.

## What needs the lab machine

The URL session transport (`session_send`) against a running cloud server, the certificate check of the `https`
endpoint, and the cookie round trip that the shell's keychain stores.

## Remaining gaps

- No signup, profile, device or session listing calls; only sign-in and sign-out are typed.
- The session token is not stored in the keychain by this module; the caller decides where it lives.
- Refreshing an expired session is not modelled: the service sets a 30-day cookie and the caller signs in again.
