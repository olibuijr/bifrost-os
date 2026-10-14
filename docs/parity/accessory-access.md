# Accessory Access

Framework: Accessory Access, https://developer.apple.com/documentation/accessoryaccess (macOS 27).
Branch: parity/team-k-accessory-access (built on arm). Not yet verified on Midget.

## What was built

- `springboard/sdk/src/accessory_access.rs` (`bifrost_app::accessory_access`): the app-facing names for the
  accessory manager (`AccessoryManager`), the accessory object (`Accessory`, with its device descriptor, configuration
  and identity strings), the matching criteria (`MatchingCriteria`, `DeviceMatch`, `InterfaceMatch`,
  `InterfaceMatchOption`), listener events (`Event`), sessions (`Session`) and errors (`Error`). All are re-exports of
  the core model, so there is one implementation.
- `springboard/core/src/accessory_manager.rs`: `impl Display for Error`, so apps can show the error values. A test
  checks that every value has a message.
- Unit tests in the SDK module: a vendor-criteria listener hears only its vendor, open is exclusive, and a session
  needs a declared protocol.

## Rows changed

- Accessory manager, Accessory object, Matching criteria, Listener events, Errors: now reachable from apps through the
  SDK module. Status stays `partial`.
- Permission and access grant, Kernel USB enumeration: unchanged. No USB permission exists in the app manifest, and
  the sandbox binds no USB device nodes (`springboard/src/bare/app_policy.rs`).

## Not built

- Bus source: nothing calls `AccessoryManager::attach` or `detach` from the kernel, so no app receives real hotplug
  events. This needs a udev or sysfs watcher in the system layer.
- Sandbox and permission: a USB device grant in `app_policy.rs` and a grant flow like `docbroker.rs`. These are
  owner decisions on the security model and are left open.
- Session data streams: the framework's `Session` data path is modelled in `external-accessory`, not here.

## Tests

Host unit tests only, written and not run in this round (the lead builds and checks):
`springboard/core/src/accessory_manager.rs` `every_error_has_a_readable_message`;
`springboard/sdk/src/accessory_access.rs` `listener_with_vendor_criteria_hears_only_its_vendor`,
`open_is_exclusive_and_the_session_needs_a_declared_protocol`.

## REA evidence

`ev_3028a6377a32f0776badf1386fc123695c24512c7074905b013c4690380d79f4` (landing page, from the matrix row).
