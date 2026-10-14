# App Intents catalog

Mirrors the reference intent definitions (the list of intents an app declares to the system) for the BifrOSt SDK.

Code: `springboard/sdk/src/intent_catalog.rs` (`Catalog`, `CatalogError`), on top of `springboard/sdk/src/intents.rs`.
Tests: `springboard/sdk/tests/parity_sdk-complete-45.rs`, headless.

## What it does

- `Catalog::new` checks a declared intent list as one set and returns the first problem: an identifier that is not
  a method token (`a-z`, `0-9`, `-`, `_`), a duplicate identifier, a blank title, or a shortcut with a required
  parameter.
- `get`, `ids`, `intents` and `shortcuts` give the lookups that the picker and Search need, in declaration order.

## What is wired

Nothing calls `Catalog::new` on the control socket yet. `App::intents()` is still served as declared, so the checks
run only when an app builds a catalog. Search reads the `[[intent]]` tables of `manifest.toml`.

## What needs the lab machine

The picker and Search listing are checked on Midget only.
