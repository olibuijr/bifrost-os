# Disk Arbitration parity (wave 4)

Two gap rows from the Disk Arbitration matrix (`docs/parity-matrix/data/disk-arbitration.json`) are now modelled in pure
Rust in `springboard/core`: exclusive-use disk claims, and disk lookup with a description. Both are pure models. They read
no block device, run no timer and call no system service. Nothing is wired into the installer, Files or the shell yet, and
none of it has run on the VM or Midget.

## What was built

- `springboard/core/src/disk_claim.rs` (`pub mod disk_claim;` in `springboard/core/src/lib.rs`). `Claims` keeps one holder
  per BSD device name. `claim(bsd_name, owner, release)` takes a free disk at once, re-claims a disk the same owner holds,
  and takes a disk from another owner only when that holder's release gate returns `Ok`. A holder with no gate gives
  `ClaimError::Busy`. A gate that returns `Err(reason)` gives `ClaimError::Refused` and the holder keeps the disk.
  `unclaim` is owner-checked (`NotOwner`, `NotClaimed`). `is_claimed`, `owner_of` and `disappeared` (drops the claim when
  the disk leaves) complete the surface.
- `springboard/core/src/disk_events.rs` (extended, existing module). Added `find_by_bsd_name`, `find_by_volume_path`
  (matches a disk's mount point; a trailing slash is ignored, the root stays `/`, an empty path or an unmounted disk never
  matches) and `description`, which returns a sorted `BTreeMap<&'static str, String>` with `bsd_name`, `media_size`,
  `media_removable`, `media_writable`, plus `device_model` and `volume_path` when present. The snapshot-based `Disk` type
  from wave 2 is the input; no new disk type was added.

Both changes have unit tests in their own `#[cfg(test)] mod tests`: 7 in `disk_claim`, and 5 new ones in `disk_events`
(the module already had 7 from wave 2).

## Official API names mirrored

- Disk Arbitration landing page: https://developer.apple.com/documentation/diskarbitration (macOS 10.4, Mac Catalyst 13.0).
  It describes the framework as providing "mechanisms to register and block disk mount or unmount events".
- `DADiskClaim(_:_:_:_:_:_:)`: https://developer.apple.com/documentation/diskarbitration/dadiskclaim(_:_:_:_:_:_:). The
  page lists it beside `DADiskIsClaimed`, `DADiskUnclaim` and `DADiskSetOptions`, and the matrix row gives the summary
  "Claims the specified disk object for exclusive use."
- `DADiskCopyDescription`, `DADiskCreateFromBSDName`, `DADiskCreateFromVolumePath`, `DADiskGetBSDName`, from the DocC
  index of `diskarbitration/dadisk-h` (https://developer.apple.com/documentation/diskarbitration/dadisk-h). The description
  dictionary key set (volume name and path, media size, writable, removable, device model, and others) comes from the
  Disk Arbitration Constants page.

## Limits and decisions

- Claim handover is modelled on the release-callback role named in the API list (`DADiskClaimReleaseCallback`). The
  official pages I read do not state the conflict rule in words, so the rule above (a conflict asks the holder's gate;
  no gate means `Busy`) is a BifrOSt decision, not a verified Apple behaviour. The docs describing it are not fetched.
- Claim options (`DADiskSetOptions`, `DADiskGetOptions`) are not modelled. The row stays partial.
- The description is a text dictionary of the fields the installer's snapshot holds. It has no UUID, bus, protocol,
  ejectable or icon entries, and no key names taken from the framework (those names are not in this crate, per the owner
  rule).
- `find_by_volume_path` matches exact mount points. It does not resolve a path inside a volume to the volume that holds it.
- Mount, unmount, eject and rename commands, the session and run-loop row, and the Files Volumes sidebar are still gaps.

## Matrix rows closed

| Framework | Feature (exactly as in `docs/parity-matrix/data/disk-arbitration.json`) | New status | BifrOSt path |
|---|---|---|---|
| Disk Arbitration | Disk claims (DADiskClaim, DADiskUnclaim, DADiskIsClaimed) | partial | springboard/core/src/disk_claim.rs |
| Disk Arbitration | Disk description and lookup (DADisk, DADiskCreateFromBSDName, DADiskCreateFromVolumePath, DADiskGetBSDName, DADiskCopyDescription) | partial | springboard/core/src/disk_events.rs |

## REA evidence

- Disk Arbitration landing page: `ev_2d6e574a48743014d88897435db0bc9e6fad3f91fde937d8e6ab8e07dc43994e`
- DADiskClaim(_:_:_:_:_:_:) page: `ev_44fa3413c30d6676b85891924a8c5e062acb714a28a40758d0402d2b252fe3f2`
- DiskArbitration Constants page: `ev_735fcff505c2bc8c80898c91b5d2fe3f713c6a7e91f0fc4a4d121cc696ddd4c0`

## Checks

- `python3 tools/check-names.py` exits 0 on this tree.
- No build or test was run in this wave (the wave rule is no builds). The code is checked by reading only.
