# Local Authentication parity: authentication prompt state

Framework: Local Authentication (https://developer.apple.com/documentation/localauthentication), matrix row in
`docs/parity-matrix/data/local-authentication.json`.

## What was built

`springboard/ui/src/auth_prompt.rs` adds `AuthPrompt`, the state of a device-owner authentication sheet. It is a pure
model with no drawing, in the `bifrost_ui` crate.

- `AuthPrompt::new(reason, allow_fallback, max_attempts)` refuses an empty reason and zero attempts. The reason is
  trimmed.
- `submit(password_ok)` records one attempt. A correct password succeeds at once. Each wrong attempt counts down
  `attempts_left()`, and the last allowed wrong attempt sets `PromptPhase::LockedOut`.
- `cancel()` and `choose_fallback()` end the prompt. The fallback choice is refused unless the prompt offered it.
- Every action after the prompt has finished returns `PromptError::NotPrompting`.

The default limit is 5, the same as `MAX_FAILURES` in `springboard/src/shell/services/local_auth.rs`. That service still
checks the password and keeps the per-app lockout; the prompt only tracks what one sheet shows, and nothing calls it yet.
`springboard/ui/src/auth_view.rs` is the model of the biometric icon view and is separate.

Nine host unit tests cover the attempt count, lockout, cancel, fallback and input checks. The shell does not draw a
reusable sheet yet, and the row is not verified on Midget.
