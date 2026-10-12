# Wave 3: file system module models (fskit)

Branch `parity/w3-fskit`, built on `parity/integration` (waves 1 and 2). Pure Rust, std only, host unit tests.
Not wired into the shell or apps, and not yet tested on the VM or Midget. No build or test was run in this round
(the wave brief forbids builds); the tests are written and the expected values were worked out by hand.

## What was built

Three pure models in `springboard/ui/src/`, one `pub mod` line each in `springboard/ui/src/lib.rs`.

- `springboard/ui/src/fs_volume.rs`: a volume with an identifier and name, and a tree of items (file, directory,
  symlink, socket, fifo, character device, block device). Names follow the file name rules (1 to 255 bytes, no
  separator or NUL, not `.` or `..`). Operations: create under a directory, lookup, children, absolute path, remove
  (empty directories only, root protected), used bytes. Directory sizes are forced to 0.
- `springboard/ui/src/fs_probe.rs`: the match grade a probe reports (not recognized, recognized, usable but limited,
  usable), the probe result with its container identifier, `best_mountable` (highest grade that can mount, first of
  equal grade wins), and the container status (not ready, ready, active, blocked, with an error text).
- `springboard/ui/src/fs_maintenance.rs`: a request queue for check, format and single-volume mount requests. One
  active request per resource, FIFO `next_queued`, phases queued, running, succeeded, failed (with reason), cancelled,
  and a per-request message log.

Each module has a `#[cfg(test)] mod tests` block (7 test functions in `fs_volume.rs`, 7 in `fs_probe.rs`, 7 in
`fs_maintenance.rs`).

## Official API names mirrored

Source: DocC JSON (`python3 -I tools/parity/docc.py fskit ...`) and the rendered page.

- `FSVolume` (`init(volumeID:volumeName:)`, `volumeID`, `name`): https://developer.apple.com/documentation/fskit/fsvolume
- `FSItem`, `FSItem.ItemType`, `FSItem.Attributes` (size only): https://developer.apple.com/documentation/fskit/fsitem
- `FSFileName`: https://developer.apple.com/documentation/fskit/fsfilename
- `FSProbeResult` (`recognized`, `usable`, `usableButLimited`, `notRecognized`, `containerID`, `name`, `result`): https://developer.apple.com/documentation/fskit/fsproberesult
- `FSMatchResult` (`usable`, `usableButLimited`, `recognized`, `notRecognized`): https://developer.apple.com/documentation/fskit/fsmatchresult
- `FSContainerStatus` and `FSContainerState` (`active`, `blocked`, `notReady`, `ready`, `status`, `state`): https://developer.apple.com/documentation/fskit/fscontainerstatus
- `FSClient` (`mountSingleVolume(resource:bundleID:options:completionHandler:)`): https://developer.apple.com/documentation/fskit/fsclient
- `FSManageableResourceMaintenanceOperations` (`startCheck(task:options:)`, `startFormat(task:options:)`): https://developer.apple.com/documentation/fskit/fsmanageableresourcemaintenanceoperations
- `FSTask` log and completion, `FSOperationID`: https://developer.apple.com/documentation/fskit (Tasks and operations section)

Not mirrored, and why: the `FSUnaryFileSystem` module protocol (needs a kernel or FUSE binding, which the pure-Rust rule
forbids), the typed `FSError` domain (errors are text here), extents and blockmaps, resource kinds, the options
dictionaries, and the `fetchInstalledExtensions` call.

## REA evidence

- Landing page `https://developer.apple.com/documentation/fskit`: `ev_5b355f13b681b4ab14ba9c0ccb374c9a9c21947b3c62c730c5d1cc9940210a53`
  (rendered headings include Volumes, Items, Containers, Maintenance and management, Tasks, FSClient, FSProbeResult).

## Matrix rows closed

Framework slug `fskit`. Feature names are exactly as written in `docs/parity-matrix/data/fskit.json`. New status
`partial`, because the three models are not wired to the shell or apps.

| framework | feature | status | BifrOSt path |
|---|---|---|---|
| fskit | Volumes and items | partial | springboard/ui/src/fs_volume.rs |
| fskit | Containers and probing | partial | springboard/ui/src/fs_probe.rs |
| fskit | Client interaction and maintenance | partial | springboard/ui/src/fs_maintenance.rs |

Not closed: "Unary file system extension (user-space module)" (needs a kernel or FUSE binding, not pure Rust).

## Gaps that remain

- No SDK wrapper: apps cannot reach these models yet. The file panel does not show mounted volumes
  (`docs/bifrost-ui.md`, file panel section).
- No runner: nothing drives the maintenance queue or the probe step, and no mount happens.
- Volumes hold only names, types and sizes; no attributes, extended attributes or open and read handlers.
