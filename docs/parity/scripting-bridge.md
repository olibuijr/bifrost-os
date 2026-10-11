# Scripting Bridge parity: what BifrOSt built

Official API: https://developer.apple.com/documentation/scriptingbridge (Mac Catalyst 13.0+, macOS 10.5+).
Matrix row: `docs/parity-matrix/data/scripting-bridge.json`.

Official classes checked (DocC, `tools/parity/docc.py`):

- `SBObject`: `property(withCode:)`, `property(with:code:)`, `elementArray(withCode:)`, `setTo(_:)`, `get()`, `lastError()`.
- `SBElementArray`: `object(withName:)`, `object(withID:)`, `object(atLocation:)`, `array(byApplying:)`.
- `SBApplicationDelegate`: `eventDidFail(_:withError:)`.

## Evidence

- REA page capture of the landing page: `ev_17fa3380a6666bef213625b91e2abcb65a786d5b82e9c31d5304a03b04bb45ba`
  (title "Scripting Bridge | Apple Developer Documentation"; the page says the framework "dynamically implements an
  Objective-C bridge to OSA-compliant applications" and generates classes for "properties, elements, commands").
- The matrix already held `ev_c5d179ada5d816aa92356565b0eea84198d86fbb156b5f7d28b04153ee82c2d8` from an earlier capture.

## What was built

`springboard/ui/src/scripting.rs` (pure std, no dependencies; exported as `bifrost_app::controls::scripting`):

- `Dictionary`, `Class`, `Property`, `ElementArray`, `Kind`: a machine-readable description of an app's scriptable
  classes. `Dictionary::describe_xml` writes it as XML text with `property` (type and `rw`/`r` access) and `element`
  lines. This mirrors the role of the dictionary file, not its exact format.
- `Object`: a scriptable object with its class, id, name, property values and element arrays. The host sets state with
  `put` and `add`. Reads go through `property` and `children`.
- `Target`: the application as seen by a caller.
  - `get` and `set` address a property by a path of `Step`s. `set` refuses read-only properties and values of the
    wrong kind. Setting `Value::Null` clears a value.
  - `children` lists an element array in order.
  - `Locator::Name`, `Locator::Id` and `Locator::Index` pick the object in an array. Index is 0-based.
  - An undeclared class, property, element array or object returns a typed `ScriptError` (`ErrorKind`).
  - `last_error` holds the error of the last call and is cleared by a success. `on_error` is the failure delegate and runs
    once per failed call.

Tests: six in `springboard/ui/src/scripting.rs` (`dictionary_xml_...`, `reads_declared_properties_...`,
`writes_respect_access_and_kind`, `elements_are_found_by_name_id_position_and_nested_paths`,
`element_arrays_must_be_declared_...`, `the_delegate_hears_each_failure_once_...`). They were written but not run,
per the build-free stage.

## Row status

- Class mapping from a dictionary: `partial`. The model and XML exist; no proxies, and no app serves its dictionary.
- Properties and elements: `partial`. Get and set are checked, but no app routes its control methods through them yet.
- Element lookup by name, id or position: `partial`. No `array(byApplying:)` filtering.
- Error reporting: `partial`. Last error and delegate exist per target, not per object.
- The framework stays `partial`.

## Remaining gaps

- Wire `Target` into an app's `App::rpc` handler and publish `Dictionary::describe_xml` as a control method, for example
  `app.dictionary`, so callers stop reading ad hoc method lists.
- Keep last-error state per object, as `SBObject.lastError()` does.
- Element filtering (`array(byApplying:)`) and per-element message mapping.
- Caller-side proxies: nothing generates typed clients from a dictionary.
- Socket access across accounts: the control socket still admits only the owning UID.
- Launch flags and an explicit activate call for `open_with`.
- Locator base: Apple's index base for `object(atLocation:)` is not stated in the official text, so the model uses 0.
  Confirm it against a running scriptable app before an app depends on it.
