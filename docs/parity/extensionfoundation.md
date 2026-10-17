# ExtensionFoundation parity

Official documentation: https://developer.apple.com/documentation/extensionfoundation (macOS 13.0).

REA evidence of the rendered landing page: `ev_e615caa5d29ed49a04e88768d64f3924695d174c1e3d217708ca4b8ff48f3aed`.
The matrix row that existed before this work cites `ev_d1c4f8ce36f377938b28696b41679f6d283efbe97d6190b90b0558dc854c7623`.

Apple API shapes were read with `tools/parity/docc.py` for `extensionfoundation`, `extensionfoundation/appextensionpoint`,
`extensionfoundation/appextensionidentity` and `extensionfoundation/appextensionprocess`.

## What was built

All three pieces are pure Rust with std only. They do not start processes, read packages or touch the network.

| Module | Mirrors | What it does |
|---|---|---|
| `springboard/ui/src/extension.rs` | `AppExtensionPoint`, `AppExtensionIdentity`, the discovery part of `AppExtensionPoint.Monitor` | `Point` (identifier under the owner's bundle namespace, `Scope::Any` or `Scope::SameDeveloper`), `Identity` (bundle, point, name, enabled; equality and hashing ignore name and state), `Registry` (`declare`, `install`, `uninstall`, `set_enabled`, `discover`, `matching`). `manifest_toml` writes `[[extension_point]]` and `[[extension]]` tables. |
| `springboard/core/src/extension_process.rs` | `AppExtensionProcess` | `ProcessTable`: `acquire` reuses a live process for a bundle and point, `running` is reported by the launcher, `connect` is refused until the process runs, `send`, `take_inbound`, `reply`, `receive` move messages in order, `invalidate` closes one connection, `exit` drops the process and its connections. |
| `springboard/sdk/src/extension.rs` | the app-facing names | Re-exports the two modules as `bifrost_app::extension`, and `manifest(points, identities)` writes the text an extension package puts in `manifest.toml`. |

Changed registrations: one `pub mod` line each in `springboard/ui/src/lib.rs`, `springboard/core/src/lib.rs` and `springboard/sdk/src/lib.rs`.

## Tests

Each module has its own `#[cfg(test)] mod tests`:

- `ui/src/extension.rs`: 8 tests (bundle identifier rules, point namespace, duplicate points, install rules for unknown points and scope, discovery order and enabled filter, reinstall and uninstall, identity equality, manifest text).
- `core/src/extension_process.rs`: 5 tests (key reuse, launching versus running, ordered messages in both directions, invalidating one connection, exit and fresh acquire).
- `sdk/src/extension.rs`: 3 tests (host and extension resolve through the registry into a process, manifest order, invalid identifiers).

Not run: per the task, no cargo build or test was run in this stage. Another stage builds and tests the crates.

## Mapping to Apple's names

- `AppExtensionPoint` and its `Scope` attribute: `Point` and `Scope`. Apple's `init(identifier:)` has no owner check; here the identifier must sit under the owner's bundle identifier.
- `AppExtensionIdentity`: `Identity`. Apple's `==` compares the bundle identifier and extension point identifier, and `hash(into:)` hashes the same pair; `Identity` does the same.
- `AppExtensionProcess`: `ProcessTable` with `ProcessId` and `ConnectionId`. Apple's `makeXPCConnection()` returns an XPC connection; here a connection is an in-memory pair of queues because no XPC transport exists on this system.
- `AppExtensionProcess.Configuration` and the synchronous and asynchronous `init(configuration:)`: `Key` and `acquire`. There is no async variant.

## Remaining gaps

1. (Done 2026-10-17) `bifrost_ui::extension_manifest::scan` reads the `[[extension_point]]` and `[[extension]]` tables of installed manifests into the registry; `shell::extension_host::Host::installed` does it for the installed packages, and a switched-off extension is not offered. It is read when a host is created (the shell's snapshot, or Files at launch), not on install or removal.
2. (Done 2026-10-17) `springboard/src/shell/extension_host.rs` starts the package's app binary in extension mode on the first call, marks the process running in the `ProcessTable` after `extension.ping`, and stops it after 30 s idle. It runs unsandboxed: `bare/app_policy` does not bind the socket directory.
3. (Done 2026-10-17) `bifrost_app::extension_service::serve` is the extension side: it answers on the control socket with a `Service` closure-like handler (`FileProviderService` for file providers). The in-memory queues of the `ProcessTable` are not used for transport; the socket is.
4. Share-sheet extensions installed by apps: the share menu (`springboard/sdk/src/share.rs`, shell side in `springboard/src/shell/sharing.rs`) does not read the registry.
5. Extension point attributes for user interface and enhanced security are not modelled, and there is no availability update stream (Apple's deprecated `availabilityUpdates`, and `AppExtensionPoint.Monitor`).
6. The `ExtensionPointDefining` protocol has no equivalent. A point is declared with a value, not a type conforming to a trait.

## Adding support for app extensions (team A5 reconciliation)

The row "Adding support for app extensions to your app" was still marked gap with no code. The extension model from wave 1
(`springboard/ui/src/extension.rs`, `springboard/core/src/extension_process.rs`, `springboard/sdk/src/extension.rs`) is the
implementation, so the row is partial now. Built on parity/team-a-05; not yet verified on Midget.
