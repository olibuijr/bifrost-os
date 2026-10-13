# SDK parity: System group (sdk-system, round 75)

Scope: `python3 tools/bifrost.py pc parity gaps --group System`. Rows were checked against the code first. Many rows
listed as gaps already have a model in the SDK (power and thermal state, process info, file stat, entropy, the
notify bus, keychain groups, file coordination, logging), so their matrix entries are stale and were not edited here.

Code is written from the row text and the existing code. The shipped reference was not measured with the REA tools
in this round; the API names come from the matrix row text. The matrix rows are not edited (the lead does closures).

## Rows advanced

### System (Partial, 50%): volume capacity
- API: `statvfs`-style capacity of the filesystem holding a path (total, free, available bytes).
- Paths: `springboard/core/src/volume.rs` (model and the only `statvfs` declaration), `springboard/sdk/src/volume.rs`
  (re-export for apps).
- Adopt and delete: `springboard/src/bare/sys.rs` `disk_space` is now a thin wrapper over `bifrost_core::volume::capacity`.
  The shell's private `statvfs_t` struct and its extern block are deleted. The five shell callers
  (`src/files/fs.rs`, `src/shell/storage.rs`, `src/net/share.rs`, `src/bin/shell.rs`) keep the tuple signature.
- Done: unit tests for the fragment and block size units, the used count and fraction, the root filesystem, and missing
  or NUL paths.
- Still needs Midget: the Storage page and Files free-space readouts are unchanged in behaviour; check them on device.
- Still a gap: the row's "typed Errno for apps" is separate; the SDK file type and path type rows are untouched.

### Core Services / Identity Services (Partial): user and group API for apps
- API: real and effective user and group IDs, supplementary groups, local account and group records, and the
  groups a user belongs to.
- Paths: `springboard/core/src/identity.rs` (IDs through `getuid`-family calls, passwd and group parsers, lookups),
  `springboard/sdk/src/identity.rs` (file reads and the current user).
- Done: parser tests (comments, directory-service markers, bad lines, empty members), lookup tests, and a test that the
  real user ID matches the kernel status record.
- Still a gap: accounts from a directory service are not listed; there is no password or identity change from apps.
  The `identity` name in the SDK does not collide with `src/term/identity` (shell-local).
- Still needs Midget: nothing user-visible; check that a sandboxed app reads its own ID.

### System Configuration / CFNetwork reachability (Partial): reachability test for a named host
- API: whether the machine has a default route, and a TCP probe of a host and port with a timeout.
- Paths: `springboard/core/src/reachability.rs` (route table parser, outcome model, probe),
  `springboard/sdk/src/reachability.rs`.
- Done: tests for the default-route parse (header and subnet-only tables), outcome classification, and loopback
  probes (listening port reachable, closed port refused).
- Still a gap: the probe is a TCP connect, not the system's route flags for a named host. It blocks while the name is
  resolved, so apps must call it off the main loop. No change events, interface name or reachability observer.
- Still needs Midget: a probe to an internal host from the Midget Lab VLAN and the default-route result on Wi-Fi and
  on the wired port.

## Not touched in this round
- Local Authentication, Keychain access control, Security (key objects and crypto transforms), Network Extension,
  Dispatch sources, OSLog signposts. These need shell or wider design work and were left for later rounds.

## Files touched
- `springboard/core/src/lib.rs` (three `pub mod` lines at the end)
- `springboard/sdk/src/lib.rs` (three `pub mod` lines after `file_stat`)
- `springboard/src/bare/sys.rs` (`disk_space` wrapper; private statvfs deleted)
- New: `springboard/core/src/{volume,identity,reachability}.rs`, `springboard/sdk/src/{volume,identity,reachability}.rs`

Not built or tested in this round (brief rule: the lead builds).
