# App install coordination and uninstall

Code: `springboard/src/shell/store/install.rs` (package checks, unpacking, uninstall), `store/coordination.rs` (the install
queue), `store/mod.rs` (App Store UI and the Updates tab).

## Verification (`install.rs`)

1. The package's SHA-256 is checked.
2. A detached ML-DSA-65 signature is checked against a known publisher key, with the package context string
   (`install_package_signed`). A package with no valid signature from a known publisher is discarded before anything is
   unpacked.
3. The archive is read in memory. Unsafe paths are rejected, and `manifest.toml` is validated.
4. The files go to `apps/<id>/<version>/`, and the `current` link is flipped. One previous version is kept for rollback.
5. The registry `apps/installed.json` is updated.

## Coordination queue (`coordination.rs`)

- One queue serves the install daemon, the authorisation agent, the coordinator and the progress reporter.
- A request is checked against the session owner. Installs run one at a time on a worker thread.
- Every phase change and byte count becomes an event. The App Store Updates tab and the Today widget drain these events.
- `cancel` stops a queued or running job. The coordinator does no I/O itself.

## Uninstall

- In a user session, `uninstall` asks privd (`store_uninstall`). Without privd (or as root), `uninstall_local` removes the
  registry entry and the `apps/<id>` package folder.
- The app's data directory is kept.
- Settings > Storage removes packages after a confirmation.

## Diagnostics

Install failures, from the app and the system, are listed on the diagnostics page.
