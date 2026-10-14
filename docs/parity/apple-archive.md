# Apple Archive parity: tar packages (sdk-rest-04)

## What it mirrors

No direct reference equivalent exists for the tar reader. It is the read side of the archive family: the package store
installer reads ustar packages, and `bifrost_app::tar_package` exposes the same reader to apps.

## What is wired

- `springboard/sdk/src/tar_package.rs`: `read` (every member with contents), `index` (headers only), `extract_from`
  (writes members below a folder, re-checking every path).
- The install path in `springboard/src/shell/store/install.rs` uses the same `read_tar`, `index_tar` and `extract_from`.
- Tests: `springboard/sdk/tests/parity_sdk-rest-04.rs`.

## What needs the lab machine

- Installing a real package through the store install flow on Midget, which is not verified here.

## Not covered

- Encrypted archives (no AEAD cipher crate is a direct dependency).
- Ownership, times and extended attributes; pax headers are refused.
- Apple Archive's own container format. Compressed archives use ZIP in `springboard/sdk/src/archive.rs`.
