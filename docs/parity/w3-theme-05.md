# Wave 3, theme-05: presentation context and web authentication sessions

## What was built

`springboard/core/src/auth_session.rs` (pure std model, no window, browser or socket):

- `PresentationContext` anchors an authentication request to an open window. `validate` returns the anchor or one of
  two errors (not provided, not an open window).
- `ControlSize` (Mini, Small, Regular, Large, ExtraLarge) gives the side of the biometric state icon in points. The
  values (16, 20, 24, 32, 40) are BifrOSt's choice; the platform does not publish pixel sizes.
- `WebSession` is the lifecycle of a web authentication session: `can_start`, `start`, `cancel`, `complete` (only a
  redirect with the session's callback scheme counts) and `outcome` (callback URL, or the canceled-login error).
- `SessionError` mirrors the error codes canceled login, presentation context not provided and presentation context
  invalid, plus two BifrOSt cases (not running, callback mismatch).

Apple's official pages list these names as the model: `LAAuthenticationView` (init with context and with control
size), `ASWebAuthenticationSession` (`init(url:callback:completionHandler:)`, `prefersEphemeralWebBrowserSession`,
`canStart`, `start()`, `cancel()`, `presentationContextProvider`) and `ASWebAuthenticationSessionError` (`canceledLogin`,
`presentationContextNotProvided`, `presentationContextInvalid`). The rendered LAPresentationContext page was not found
in DocC; the presentation context here follows the role of `ASWebAuthenticationPresentationContextProviding`.

Not built: the browser handoff, the auth icon view itself, the biometric state source, and any SDK wiring. Those stay
gaps.

## REA evidence

- `ev_7abef1cda51f3ea7749fcb6ad56d3b78bf169002652f36b005e3f98e17001913`: LAAuthenticationView
- `ev_583c16ba357795eeb021c93ebe833558b89dd1ec1fe6ca034dff57377c8deda3`: ASWebAuthenticationSession
- `ev_99db97d8c510096f10a324f9a0865fb6bdb822103638ba1369c209d24c9fb3eb`: ASWebAuthenticationSessionError

## Matrix rows closed (status partial)

| framework slug | feature | BifrOSt path |
|---|---|---|
| local-authentication-embedded-ui | Presentation context data type | springboard/core/src/auth_session.rs (PresentationContext) |
| local-authentication-embedded-ui | Sizing the view with a control size | springboard/core/src/auth_session.rs (ControlSize) |
| authentication-services | Web authentication sessions | springboard/core/src/auth_session.rs (WebSession, SessionError) |

## Tests

Six behaviour tests in `auth_session.rs`: context validation, control size ordering, scheme parsing, start rules,
callback completion and finished-state rules, cancel outcome, and a failed start leaving the session idle.
No build was run, per the task rules.
