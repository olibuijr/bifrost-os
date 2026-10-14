# Authentication Services: parity round (team N)

Matrix data: `docs/parity-matrix/data/authentication-services.json`. Earlier waves are in `w3-`, `w5-` and `w9-authentication-services.md`.

## What this round adds

- Callback routes for web authentication sessions: `springboard/core/src/auth_callback_routes.rs` (`CallbackRoutes`).
  An app registers one callback scheme. Reserved web and browser schemes (`https`, `http`, `file`, `about`, `data`,
  `javascript`, `blob`, `chrome`) are refused, a scheme has one owner, and `deliver` hands a redirect only to the app
  that started the session. Unit tests cover registration, conflicts, removal and delivery.

## Rows

| Framework | Feature | Status | BifrOSt path |
|---|---|---|---|
| Authentication Services | Single sign-on in a web browser app | partial | springboard/core/src/auth_callback_routes.rs |
| Authentication Services | Credential import and export | partial | springboard/ui/src/credential_exchange.rs (row was gap; wave 9 model) |

## Not done

- The browser app does not broker sign-in for other apps, and the shell does not route redirects through `CallbackRoutes` yet.
- No sign-in sheet that presents the page in an isolated session.
- No passkey authenticator, key generation or credential registration.
- No shared credential store, AutoFill or save-password prompt.
- No CXF interoperability.

## Status

Not built or tested in this round beyond host unit tests. Built on the parity/team-n-authentication-services branch; not yet verified on Midget.
