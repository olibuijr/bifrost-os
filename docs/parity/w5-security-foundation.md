# Wave 5: Security Foundation (authorization rights)

## What was built

- `springboard/ui/src/access_control.rs`: `RightStore::obtain` (authorizes the named rights in the order asked, granting
  a missing right only when `ObtainFlags::interaction_allowed` is set and the `Presence` satisfies user presence),
  `RightStore::invalidate_credentials` (drops every right), and the types `ObtainFlags` and `ObtainError`. A refused
  request grants nothing: every name is checked and every grant planned before the store changes, and the capacity
  check runs before the first grant. Seven behaviour tests in the module's `tests`.
- `springboard/sdk/src/authorization.rs` (new, one `pub mod authorization;` line in `springboard/sdk/src/lib.rs`): the
  app-facing `Authorization` object that holds a `RightStore` and delegates `obtain`, `state` and
  `invalidate_credentials`. Five behaviour tests.

The rights live in the app's `Authorization` object. The shell does not enforce them yet; the manifest permission
(`springboard/src/bare/app_policy.rs`) is still the only grant the shell enforces at launch.

## Apple API mirrored (official documentation)

- `SFAuthorization` class, "restrict a user's access to particular features in your Mac app or daemon".
  https://developer.apple.com/documentation/securityfoundation/sfauthorization
- `obtain(withRights:flags:environment:authorizedRights:)` (macOS 10.10): "Authorizes and preauthorizes rights to access
  a privileged operation and returns the granted rights." It throws.
  https://developer.apple.com/documentation/securityfoundation/sfauthorization/obtain(withrights:flags:environment:authorizedrights:)
- `obtain(withRight:flags:)`: "Authorizes and preauthorizes one specific right."
- `invalidateCredentials()`: "Prevents any rights that were obtained by this object from being preserved."

Mapping: `obtain` maps to `RightStore::obtain` and `Authorization::obtain`; `invalidateCredentials` maps to
`RightStore::invalidate_credentials` and `Authorization::invalidate_credentials`. `ObtainFlags` covers only
interaction. The environment parameter, preauthorization and flag members other than interaction are not modelled.
The official page states the authorization services API is not supported inside an app sandbox, because it allows
privilege escalation. The app object here grants nothing the shell has not allowed, so the model is not a sandbox
escape, but the shell-side enforcement is still missing.

## REA evidence

- Official SFAuthorization page: `ev_300499ce44a15987403130dc9be548cfd66b43d1661abee42895ef557dd9abb8`
  (`tools/parity/rea-page.sh https://developer.apple.com/documentation/securityfoundation/sfauthorization`).
- The `obtain` symbol page and the class page were read with `tools/parity/docc.py`; no separate REA id was taken for
  them.

## Matrix rows closed

Framework slug: `security-foundation`. Status set to `partial`, because the rows have no environment model and no
shell enforcement.

| Framework slug | Feature (exactly as in data/security-foundation.json) | New status | BifrOSt path |
|---|---|---|---|
| security-foundation | SFAuthorization object and rights (obtain(withRights:flags:environment:authorizedRights:), obtain(withRight:flags:), authorizationRef()) | partial | springboard/ui/src/access_control.rs (RightStore::obtain); springboard/sdk/src/authorization.rs (Authorization::obtain) |
| security-foundation | Authorization environment and flags (init(flags:rights:environment:), authorization(with:rights:environment:)) | partial | springboard/ui/src/access_control.rs (ObtainFlags, interaction_allowed only) |
| security-foundation | Credential invalidation (invalidateCredentials()) | partial | springboard/ui/src/access_control.rs (RightStore::invalidate_credentials); springboard/sdk/src/authorization.rs (Authorization::invalidate_credentials) |

Not closed: "Per-app permission enforcement" (os) stays as it is. The shell enforces the manifest grants at launch, and
an app-side query of them still needs a shell call. "Secret storage gated by an app permission" (partial) is unchanged.

## Gaps left

- The shell does not enforce `Authorization` rights; a granted right exists only in the app's object.
- No user prompt is wired to `Presence`; the caller reports presence and the model answers.
- The environment parameter and preauthorization flags are not modelled.
- `authorizationRef()` has no BifrOSt counterpart and is not planned here.

## Verification

Not built or run, per the wave rules. The Rust was checked by reading only. The `tests` modules hold hand-worked values
(for example, a lifetime of 60 seconds granted at 1000 is valid at 1059 and expired at 1060; the capacity check
refuses the 257th right).
