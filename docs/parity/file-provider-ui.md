# File Provider UI parity

Official reference: https://developer.apple.com/documentation/fileproviderui (framework: "Add actions to the document
browser's context menu"; macOS 10.15+). The framework lets a File Provider extension add actions to the long-press
menu of its own items in the system document browser, and shows a custom view controller that finishes each action.

## What was built

Two pure models in `bifrost-ui` (std only, no unsafe, no hardware):

- `springboard/ui/src/fileaction.rs`: the action side.
  - `ActionId`: a typed identifier, `owner` plus `name`, each non-empty ASCII letters, digits, `-`, `_` or `.`.
  - `FileAction`: id, title, the uniform type identifiers it accepts (`uttype`), and an optional `max_items`.
  - `ActionRegistry`: registers actions (rejects duplicates and unknown or empty type lists), looks them up,
    unregisters one action or every action an app owns, and returns the actions that apply to a selection.
    An action applies when every selected item conforms (through `uttype::conforms`) to one of its accepted types
    and the selection fits `max_items`. An empty selection matches nothing.
- `springboard/ui/src/actionrequest.rs`: the request behind a chosen action.
  - `ActionRequest` carries the action and the item identifiers. The hosted view may `show_error` (the request stays
    pending) and must finish it once: `complete()`, or `cancel(None)` for a user cancel, or `cancel(Some(msg))` for a
    failure. A finished request refuses further changes with `RequestError::Finished`.

Both modules are registered with one `pub mod` line each in `springboard/ui/src/lib.rs`. Unit tests cover identifier
validation, registration errors, type conformance matching, the all-items rule, `max_items`, owner removal and
request lifecycle transitions.

## Official API names mirrored

| BifrOSt | Official name |
|---|---|
| `ActionId` | `FPUIActionIdentifier` |
| `ActionRegistry` / `FileAction` | the actions listed by the document browser for a selection (Adding Actions to the Context Menu) |
| `ActionRequest` | `FPUIActionExtensionContext` (`completeRequest()`, `cancelRequest(withError:)`) |
| `ActionRequest::show_error` | `FPUIActionExtensionViewController.prepare(forError:)` |
| `Outcome::UserCancelled` / `Failed` | `FPUIExtensionErrorCode.userCancelled` / `.failed` (as a value, not an error domain) |

Official facts used: the landing page and topic list come from the DocC data (`tools/parity/docc.py fileproviderui`),
and the rendered page text was checked with REA.

## REA evidence

- `ev_fc9aa660136df59bc6974c6454bab6914a0276607be52d42a0befa7d7f89793a`: rendered landing page, this round.
- `ev_1fac05280e07c30aa14c90276ef509027811b6f154decf758afd85789940401c`: landing-page record from the earlier matrix row.

## Status changes

In `docs/parity-matrix/data/file-provider-ui.json`:

- "Context-menu actions in the document browser": gap to partial.
- "Action extension context: complete or cancel": gap to partial.
- "Action identifiers": partial (typed and validated, now with a registry).
- Framework status: gap to partial.

Rows left unchanged: the action extension view controller (still gap, no hosted process), the error codes and
authentication hand-off (n/a), the Files operations and provider domain rows (partial, as before).

## Remaining gaps

1. No consumer of the registry. The Files browser and the shell do not call `actions_for`, so apps cannot add actions
   to Files or to another app's items.
2. No hosted extension process that shows an app's custom action UI over the browser and completes or cancels a
   request. `ActionRequest` models only the lifecycle.
3. No provider domain model that lists an app's own files as a browsable location.
4. No system-wide registry that delivers action identifiers from apps to the shell.
5. No authentication-error hand-off for file actions against remote or cloud providers.

## Not verified in this round

This round did not build or run tests (the stage that builds is separate). The new modules have not been compiled.
