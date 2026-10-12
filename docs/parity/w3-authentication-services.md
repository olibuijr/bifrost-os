# Wave 3: Authentication Services (partial)

Two gap rows of the Authentication Services framework are now partial. Both are pure Rust models in `bifrost-ui`
(std only, no hardware, no network). Nothing is wired into apps or the shell yet, and the tests have not been run
(this wave does not build).

## What was built

- `springboard/ui/src/webauth_session.rs`: the web sign-in session. A `WebAuthSession` checks the start URL (http or
  https with a host) and the callback (a custom URL scheme, or an https host and path), holds the state (idle,
  running, finished with the callback URL, failed as canceled or without a presentation context), and accepts only
  the redirect it waits for. The `SessionRouter` gives each redirect to the oldest running session that accepts it,
  so a second app that registered the same scheme never receives it. Helpers `scheme_of` and `host_and_path` parse
  URLs without a crate.
- `springboard/ui/src/verification_codes.rs`: one-time codes. `extract_code` finds the code in a message: a run of 4
  to 8 digits, taken after the first code word (code, passcode, verification, otp), or the only run in the text.
  `DeliveredCodes` holds delivered codes for a lifetime, lists the active unconsumed ones, consumes a code once for
  a named process, and prunes expired codes.

## Official API names mirrored

Authentication Services, web authentication sessions:
`ASWebAuthenticationSession`, `init(url:callback:completionHandler:)`, `ASWebAuthenticationSession.Callback`
(custom scheme and https cases), `prefersEphemeralWebBrowserSession`, `canStart`, `start()`, `cancel()`,
`presentationContextProvider` (modelled as the `has_context` flag), `additionalHeaderFields`,
`ASWebAuthenticationSessionError.Code` (canceled and no presentation context).
Source: https://developer.apple.com/documentation/authenticationservices/aswebauthenticationsession

Authentication Services, one-time codes:
`ASDeliveredVerificationCodesManager`, `consumeOneTimeCode(_:)`, `oneTimeCodes(preferredDuration:anchor:)`
(modelled as `DeliveredCodes::active` with a `within` window), `ASDeliveredVerificationCodesManager.VerificationError`
(not delivered, already consumed, expired). The text extractor follows the one-time code text content type; its rules
are ours and are written in the module.
Source: https://developer.apple.com/documentation/authenticationservices/asdeliveredverificationcodesmanager

## REA evidence

- `ev_4e5fe1734b96de3aff8dad497676e4e0b128ad63a16c98e3dcb93900727ed4f3`: ASWebAuthenticationSession official page
  (the session shows the domain and asks to proceed; the callback goes back through the completion handler to the
  calling session only).
- `ev_c9564825e52cbcd8c01f2e6ea92705ba630077fdc049725e559274fb926826c3`: ASDeliveredVerificationCodesManager official
  page (codes come from many sources, such as mail and text messages; `consumeOneTimeCode` and the stream).
- `ev_eb4e8515f63dbb5347ffb186beb68ad982580d05bf3ac09772095c773e2d5156`: Authentication Services framework matrix
  entry (from the matrix data, not re-recorded in this wave).

## Matrix rows closed

| framework slug | feature (as written in the data file) | status | BifrOSt path |
|---|---|---|---|
| authentication-services | Web authentication sessions | partial | springboard/ui/src/webauth_session.rs |
| authentication-services | One-time passcodes | partial | springboard/ui/src/verification_codes.rs |

## Not built

- Passkeys and FIDO2 need public-key cryptography (elliptic curve signatures) that std does not provide; left as gap.
- Credential import and export (CXF) and an AutoFill provider need a store and UI beyond a pure model; left as gap.
- Authorization requests (controller and results) could be a pure model, but no sign-in provider exists in
  BifrOSt to use it; left as gap.

## Tests

`#[cfg(test)]` modules in both files, 8 tests in `webauth_session.rs` and 7 in `verification_codes.rs`, with expected
values worked out by hand from the rules in the code. Not run in this wave.
