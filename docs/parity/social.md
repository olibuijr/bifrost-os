# Social parity (round 1)

Framework: Social, https://developer.apple.com/documentation/social (macOS 10.8+, Mac Catalyst 13.0+).
Official overview, REA evidence of the rendered page: `ev_480f845a1c91c745b77162d082f5325cd9a09bcf497f3ef55b819f71e29e6435`
(the matrix row also keeps the earlier capture `ev_d8070e35a0b4a4bf77a71cd477acdf308f271f04addcec3639691b3fb911a8f3`).
Official symbol pages read with `tools/parity/docc.py`: `social/slrequest`, `social/slcomposeviewcontroller`,
`social/slrequestmethod`, `social/slcomposeviewcontrollerresult`.

The framework lists two composition classes, one request class, and social service types. The framework
says the request class is "a template for creating HTTP requests". This round builds the parts that do not need a
social network or a signed-in account.

## What was built

| Module | Mirrors | What it does |
|---|---|---|
| `springboard/sdk/src/multipart.rs` | `addMultipartData(_:withName:type:filename:)`, `addMultipartData(_:withName:type:)` | `MultipartBody` collects text fields and file parts (name, type, optional file name) and writes the `multipart/form-data` bytes and `Content-Type` value. The boundary is derived from the content and changed until it does not occur in any part. Names and file names escape `"`, CR and LF. An empty type becomes `application/octet-stream`. |
| `springboard/sdk/src/social.rs` | `init(forServiceType:requestMethod:url:parameters:)` (without the service type), `preparedURLRequest()`, `SLRequestMethod` (GET, POST, DELETE, PUT) | `ServiceRequest` assembles a `URLRequest`. GET and DELETE put parameters in the query string. POST and PUT send a form body, or a multipart body when uploads were added (parameters first, then uploads). Uploads on GET or DELETE return an error. |
| `springboard/ui/src/compose.rs` | `setInitialText(_:)`, `add(_:)` (image and URL), `removeAllImages()`, `removeAllURLs()`, `SLComposeViewControllerResult` (`cancelled`, `done`) | `ComposeDraft` holds the post text, images and links. Links must be `http://` or `https://` with a host. It caps images and links at 8 each (BifrOSt's choice; the framework states no limit). `dismiss(send)` returns `Done` only for a sent, non-empty draft. `items()` gives the hand-off order: text, images, links. |

The modules are registered with one `pub mod` line each in `springboard/sdk/src/lib.rs` and
`springboard/ui/src/lib.rs`. Each has unit tests (6 in `multipart.rs`, 7 in `social.rs`, 6 in `compose.rs`).
Nothing was built or run in this round; the stage that builds the workspace runs the tests.

## Matrix status

- Multipart upload body: `parity`.
- Post contents, completion result, request assembly, compose sheet: `partial`. The models exist, but no shell
  sheet shows them and no transport posts them. The share menu still reports an app id, not a compose result.
- Social account credentials, service types and service availability: `gap` / `n/a`, unchanged.
- Framework status: `partial` (was `gap`).

## Remaining gaps

1. A compose sheet in the shell that edits a `ComposeDraft` and reports `ComposeResult`.
2. Service types (`SLServiceType*`) and `isAvailable(forServiceType:)`. Named third-party networks are `n/a`; an
   availability check would need a list of configured services, which BifrOSt does not have.
3. A signed-in account store and request signing (`SLRequest.account`). Accounts for BifrOSt Cloud live in
   `~/Projects/bifrost-cloud`, outside this repo. Signing would need a token store and keys.
4. `perform(handler:)` on a request object. `ServiceRequest::prepared_url_request` returns a `URLRequest`, and the
   caller sends it with `URLSession::data_task`.
5. Per-item share objects and delegate callbacks (see `docs/bifrost-ui.md`, "Share sheet").

# Social parity (round 2)

This round builds the two gaps that do not need a sign-in screen or a network service: the account model and its
signing, and the hand-off from a sent draft to a request.

## What was built

| Module | Mirrors | What it does |
|---|---|---|
| `springboard/sdk/src/social_account.rs` | `SLRequest.account`, the signed-in account | `AccountStore` keeps account metadata only (service type, user name, `AuthMethod`) and answers `has_service`, which is the account side of `isAvailable(forServiceType:)`. `Credentials` holds the secret; `to_secret_bytes` and `from_secret_bytes` give the keychain form, stored under `KEYCHAIN_SERVICE` and `SocialAccount::keychain_account` (`service:username`). `sign` adds `Authorization: Bearer` or an OAuth 1.0a header with HMAC-SHA1 over the RFC 5849 base string. `signed_service_request` prepares a `ServiceRequest` and signs it. |
| `springboard/sdk/src/social.rs` (`draft_request`) | the bridge from `SLComposeViewController` to `SLRequest` | Builds a form POST from a `ComposeDraft`: the text in one field, each link in another, in the order added. Images are left out, since a draft keeps only the names the shell gave them. |

`encode` in `social.rs` is now `pub(crate)` so the signing code uses the same RFC 3986 encoding as the request
builder. The HMAC is built on the SHA-1 digest in `digest::insecure`, and the base64 comes from `http_auth`.
Nothing was built or run in this round; the stage that builds the workspace runs the tests.

Unit tests: 16 in `social_account.rs` (HMAC-SHA1 against the RFC 2202 vectors, the base string for a fixed request,
URL normalization, the header and its determinism, keychain round trips, store rules, the signed request) and 2 in
`social.rs` for `draft_request`.

Known limit: the base string test uses a request whose expected base string was derived by hand from RFC 5849 3.4.1.
The signature has not been checked against a live service or a published OAuth 1.0a example that includes
`oauth_callback`. The first live check should use a service the team controls.

## Matrix status

- Social account credentials: `gap` to `partial`. The model and the signing exist; no account can be added from the
  shell, and no secret is read from the keychain by the shell.
- Compose post sheet: stays `partial`. `draft_request` exists, but no shell sheet edits a draft and sends it.
- Service availability: stays `parity` (no change to its note).
- Framework status: stays `partial`. Coverage moves from 61.1% to 66.7% because of the account row.

## Remaining gaps

1. A compose sheet in the shell that edits a `ComposeDraft` and sends it through `draft_request` (with image bytes
   from the app), then reports `ComposeResult`.
2. A sign-in flow and an account provider in the shell. Accounts for BifrOSt Cloud live in
   `~/Projects/bifrost-cloud`, outside this repo, so the shell needs an agreed way to add a social account.
3. OAuth 2 flows and token refresh. Only bearer tokens and OAuth 1.0a are modelled.
4. Per-item share objects and delegate callbacks (see `docs/bifrost-ui.md`, "Share sheet"). Unchanged in this round.
