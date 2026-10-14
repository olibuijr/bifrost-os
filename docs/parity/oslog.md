# Subsystem ownership on read (parity sdk-rest-01)

Mirrors the rule that a log subsystem belongs to the process that writes it, for the `oslog` row "Subsystem ownership on read".

## What is wired

- `springboard/sdk/src/log_ownership.rs` (`bifrost_app::log_ownership`):
  - `check_claim(registry, exe, subsystem)` returns `Ok` when a process run from `exe` may log as the subsystem,
    and a `ClaimError` (`Invalid`, `OwnedByAnother`, `Reserved`) otherwise. It applies `may_log_as` and
    `valid_subsystem` from `springboard/core/src/log.rs`.
  - `owner_of(registry, subsystem)` names the store app whose manifest declared the subsystem.
- Tests: `springboard/sdk/tests/parity_sdk-rest-01.rs`, headless, over registries built with `registry_from_manifests`.

## What remains

- The writer identity is the executable path journald records (`_EXE`). The rule is checked on read by
  `LogStore`, which still removes a refused claim itself; `check_claim` is a decision for tools, not a write-time guard.
- Nothing runs against the system journal on the lab machine yet.
