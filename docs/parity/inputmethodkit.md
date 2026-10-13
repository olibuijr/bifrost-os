# InputMethodKit parity, team J: Info dictionary registration

Branch `parity/team-j-inputmethodkit` (from `arm`). Matrix source: `docs/parity-matrix/data/inputmethodkit.json`.

## What was built

| Module | Mirrors | What it does |
|---|---|---|
| `springboard/ui/src/imk_info.rs` (`registration_from_info`) | The input method Info dictionary keys `IMKControllerClass`, `IMKDelegateClass`, `IMKModeDictionary` | Reads a parsed property list (`bifrost_core::plist::Plist`) into a validated `Registration`. Modes keep dictionary order, so the first mode is the default. A mode's label is its `label` string, else its id. |

`springboard/ui/src/lib.rs` gained one `pub mod imk_info;` line.

Errors are typed (`InfoError`): a non-dictionary root, a missing or non-string controller class, a non-string delegate
class, a mode dictionary or mode settings of the wrong type, and any `RegistrationError` from `Registration::validate`.
A blank delegate name means no delegate, as `Registration::delegate` already does.

## Choices

- The `label` key inside a mode's settings is a BifrOSt choice. Apple's documentation for `IMKModeDictionary` gives no
  settings keys in the official pages checked here, so the mode settings format is ours.
- The bundle is not read from disk. The shell or a future bundle loader parses the Info property list and passes the
  root value in.

## Matrix rows

| Feature | Status | Note |
|---|---|---|
| Input method registration and Info dictionary | partial | Parsing exists with host tests. Nothing calls it at run time. |

## Not done

- The shell does not load any input method bundle, so no input method is registered at run time.
- `Server` is not connected to the shell or to any client (see the Input method server row).
- Not verified on Midget. Host tests have not been run in this branch: the lead builds and checks it.
