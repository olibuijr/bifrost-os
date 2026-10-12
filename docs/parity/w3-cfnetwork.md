# Wave 3: CFNetwork gap rows (cookies and HTTP authentication)

Two gap rows of the CFNetwork framework are now partial. Both are pure std models in the SDK, next to `url.rs`, so
URL-loading apps can use them without a network stack. No new crate, no unsafe, no hardware access.

## What was built

- `springboard/sdk/src/http_cookies.rs` (`bifrost_app::http_cookies`): `HTTPCookie` parses `Set-Cookie` values
  (name, value, `Domain`, `Path`, `Max-Age`, `Secure`, `HttpOnly`) with RFC 6265 default-path and domain rules.
  `HTTPCookieStorage` stores cookies, replaces by name, domain and path, builds the `Cookie` request header in RFC
  6265 section 5.4 order, deletes by cookie or receive time, and purges expired cookies. `AcceptPolicy` has
  `Always` and `Never`.
- `springboard/sdk/src/http_auth.rs` (`bifrost_app::http_auth`): `URLCredential` (user, password, persistence) builds
  the HTTP Basic `Authorization` value (RFC 7617) and applies it to a `URLRequest` via `set_header`.
  `parse_www_authenticate` reads the first challenge's scheme and realm (quote-aware). `URLProtectionSpace` derives
  host, port and realm from a URL. `URLCredentialStorage` keeps one credential per space. `base64_encode` is the
  RFC 4648 encoder used by the header.
- `springboard/sdk/src/lib.rs`: one `pub mod` line for each new module.

## Official API names mirrored

- `HTTPCookie`, `HTTPCookieStorage` (`cookies`, `cookies(for:)`, `setCookie`, `setCookies`, `deleteCookie`,
  `removeCookies(since:)`, `cookieAcceptPolicy`, `HTTPCookie.AcceptPolicy`): https://developer.apple.com/documentation/foundation/httpcookiestorage
- `URLCredential` (`init(user:password:persistence:)`, `user`, `password`, `hasPassword`, `persistence`,
  `URLCredential.Persistence`): https://developer.apple.com/documentation/foundation/urlcredential
- `URLAuthenticationChallenge` (`protectionSpace`, `proposedCredential`) for the HTTP Basic case only:
  https://developer.apple.com/documentation/foundation/urlauthenticationchallenge
- `URLCredentialStorage` and `URLProtectionSpace` are named in the URLCredential page's See also list. Their own
  pages were not read in this wave.

Docs read with `python3 -I tools/parity/docc.py`: `foundation/httpcookiestorage`, `foundation/urlcredential`,
`foundation/urlauthenticationchallenge`.

## REA evidence

- HTTPCookieStorage page: `ev_7c5e8fe98146c3be66ada749d6fc8165653661a32b12ad9d53fb879f71fefa55`
- URLCredential page: `ev_8731b8e5f60a29086486ffe5998669aee15452306b9a0689cf61b5a77483c5ec`
- Matrix landing evidence for CFNetwork (already in the matrix): `ev_70eacb8c8b9b4291ebcfc455d0f876fb4f8d0c9bc057f11028bf874f7e3f7c88`

## Matrix rows closed

| Framework | Feature | Status | BifrOSt path |
|---|---|---|---|
| cfnetwork | Cookies and session storage | partial | springboard/sdk/src/http_cookies.rs |
| cfnetwork | HTTP authentication | partial | springboard/sdk/src/http_auth.rs |

Both rows stay partial. What is not done:

- The cookie storage is not yet wired into `URLSession` (`springboard/sdk/src/url.rs`). The shell HTTP client keeps
  its own jar, so no app request sends or stores cookies through this model yet.
- `Expires` dates are ignored. Only `Max-Age` gives an expiry.
- `Permanent` credentials live in memory only. There is no keychain persistence, and nothing survives a restart.
- No Digest, client-certificate or server-trust authentication. `URLAuthenticationChallenge` is represented only by
  the parsed HTTP Basic challenge. No challenge sender or completion handler exists.
- No public-suffix list, so domain cookies can be set for any domain that contains the request host.

## Tests

`#[cfg(test)]` modules are in both new files, with hand-worked values (not run in this wave, per the no-build rule):

- `http_cookies`: 9 tests (attribute parsing, default path, `Domain` widening and rejection, `Max-Age` expiry, header
  order and secure filtering, host-only and subdomain matching, expiry and replacement, remove, delete and purge,
  accept policy).
- `http_auth`: 6 tests (base64 RFC 4648 vectors, RFC 7617 Basic example, header replacement, challenge parsing with
  quoted commas, protection-space default ports, storage per space and persistence).
