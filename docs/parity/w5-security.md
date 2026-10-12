# Parity wave 5: Security

Branch `parity/w5-security`, on top of `parity/integration` (waves 1 to 4).

## What was built

One gap row was built in pure Rust: **Code signing services** (`security`, status gap). The matrix said no code
signing or signature check existed for apps.

- `springboard/core/src/code_seal.rs` (new): resource seals. A `Seal` lists each file of a program bundle by relative
  path and SHA-256 digest. `Seal::build` seals files, `Seal::verify` reports modified, missing and unsealed files as a
  `Verdict`, and `to_text` / `parse` read and write a sorted `<hex>  <path>` text form. `seal_digest` gives a SHA-256
  over the text. Path rules refuse absolute paths, `.` and `..` parts, empty parts, backslashes and NUL bytes. A path
  passed twice to `verify` counts as modified.
- `springboard/core/src/lib.rs`: one `pub mod code_seal;` line.

The model reads no disk and no signature; the caller passes the bytes. It reuses `springboard/core/src/sha256.rs`
(`digest`, `hex`, `unhex`) and adds no crate and no `unsafe`.

## Limits (why the row is partial)

- A seal is an integrity record, not a signature. BifrOSt has no signing key, so whoever can rewrite the seal file can
  recompute its digest. Authenticity needs an asymmetric signature (an Ed25519 check is not in std), which is out of
  scope for a pure-std wave.
- Nothing calls the model yet: the shell does not check installed apps against a seal, and apps have no SDK call for
  it. `springboard/core/src/codereq.rs` and `springboard/core/src/codesign_events.rs` are unchanged.
- The Apple API is a static check of code on disk with error details. The model covers the file-digest part only, not
  code requirements, entitlements or dynamic (running process) validity.

## Official API names mirrored

Source: https://developer.apple.com/documentation/security/code-signing-services (DocC via `tools/parity/docc.py`).

- `SecStaticCodeCreateWithPath` (static code object for a path)
- `SecStaticCodeCheckValidity` and `SecStaticCodeCheckValidityWithErrors` (static validation with error detail)

Mapped to `code_seal::Seal::build`, `Seal::verify` and `Verdict` respectively.

## REA evidence

- `ev_2db18a949458bc863d7a000df98f70d298b540e3074e4be4d1f8b3af911e1043`: rendered page
  https://developer.apple.com/documentation/security/code-signing-services (title "Code Signing Services | Apple
  Developer Documentation"), recorded with `tools/parity/rea-page.sh`.

## Matrix rows closed

| framework slug | feature (as in `docs/parity-matrix/data/security.json`) | new status | BifrOSt path |
|---|---|---|---|
| security | Code signing services | partial | springboard/core/src/code_seal.rs |

Rows not closed in this wave, and why:
- Software update integrity (os): a signed manifest needs an asymmetric signature (see Limits).
- Keychain at-rest protection (os), App sandbox (os), Login and session authentication (os): each needs a shell or
  daemon change, not a pure model.
- Certificate, key and trust evaluation (partial): a DER certificate parser is needed before the trust model can be
  fed; out of scope.

## Tests

`springboard/core/src/code_seal.rs` has a `tests` module with six behaviour tests: the empty-file SHA-256 value, path
sorting, valid and invalid verification, the repeated-path rule, the path refusal rules and the text round trip with
damage detection. The build and test run were not done in this wave (no builds were allowed).
