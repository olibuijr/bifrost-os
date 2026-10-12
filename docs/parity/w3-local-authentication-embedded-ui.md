# Parity wave 3: Local Authentication Embedded UI

Framework: Local Authentication Embedded UI, https://developer.apple.com/documentation/localauthenticationembeddedui
(Mac Catalyst 16.0, macOS 12.0). The official page says the framework draws one graphical view that shows the
state of biometric authentication, and that a custom authentication interface is built around that view. It gives
no sizes, colours or state names; the states below are BifrOSt's own and follow that description.

## What was built

- `springboard/core/src/auth_context.rs` (`bifrost_core::auth_context`): an authentication context for one policy
  on one device. It reuses `biometry::resolve` and `biometry::Device`. `begin` starts an evaluation and returns the
  method that answers it (biometry or passcode); `finish` records the outcome, so a biometric failure counts toward
  the lockout and a passcode result does not; `invalidate` ends the context. The phase is `Ready`,
  `Evaluating(method)`, `Succeeded(method)`, `Failed(failure)` or `Invalidated`.
- `springboard/ui/src/auth_view.rs` (`bifrost_ui::auth_view`): the authentication icon as a pure model. `ViewState`
  (Idle, Evaluating, Succeeded, Failed, LockedOut), `Event` (Start, Succeeded, Failed, LockedOut, Reset), `next`
  transition function, `icon` (icon key by state and `BiometryKind`), `ControlSize` (five steps mapped to points and
  pixels) and `AuthView`, which holds state, kind and size.
- One `pub mod` line each in `springboard/core/src/lib.rs` and `springboard/ui/src/lib.rs`.

## Apple API names mirrored

- `LAAuthenticationView` (class): `AuthView`, with `send`, `state`, `icon` and `points`.
- `LAAuthenticationView.init(context:)`: `AuthView::new(kind, size)` plus `AuthContext::begin`/`finish`. The view
  does not yet read the context itself; an SDK wrapper must forward the context's phase as `Event`s.
- `LAAuthenticationView.init(context:controlSize:)` and `controlSize`: `AuthView::new(kind, ControlSize)`.
- `LAPresentationContext`: not built. The official symbol page has no abstract, so there is no documented
  behaviour to mirror. The matrix row stays a gap.
- Context type: `AuthContext` mirrors the role of an authentication context (policy plus evaluation state). It is
  not a copy of the framework's type; the official data-type list in this framework is only the presentation
  context.

## REA evidence

- Framework landing page, rendered: `ev_49a35e9b1a8c111b19830f17f3fadd853b93e2705c3ccb2cdcfba75836b2cfb0`
  (`tools/parity/rea-page.sh https://developer.apple.com/documentation/localauthenticationembeddedui`).
- Symbol pages used through `tools/parity/docc.py`: `localauthenticationembeddedui/laauthenticationview`,
  `localauthenticationembeddedui/lapresentationcontext`.

## Matrix rows closed

Rows are from `docs/parity-matrix/data/local-authentication-embedded-ui.json`. The lead updates the matrix.

| Framework slug | Feature | New status | BifrOSt path |
|---|---|---|---|
| local-authentication-embedded-ui | Authentication view (biometric state icon) | partial | springboard/ui/src/auth_view.rs (AuthView, icon, next) |
| local-authentication-embedded-ui | Creating the view from an authentication context | partial | springboard/core/src/auth_context.rs (AuthContext); springboard/ui/src/auth_view.rs (AuthView::send) |
| local-authentication-embedded-ui | Sizing the view with a control size | partial | springboard/ui/src/auth_view.rs (ControlSize) |

Not closed: Presentation context data type (no documented behaviour to mirror).

## Limits

- No biometric sensor, prompt or secure storage is read or driven. The model answers only from the device facts it
  is given, as `biometry` already does.
- The view and the context are not connected in an SDK module yet, and no app can call them. Nothing is wired into
  the login screen (`springboard/src/shell/login.rs`).
- The sizes (16, 20, 24, 32, 40 points) and icon keys are BifrOSt choices, not Apple values.
- The tests were written by hand from the rules; they were not built or run in this wave (no builds were allowed).
  The next build must run `bifrost-core` and `bifrost-ui` tests before merging.
