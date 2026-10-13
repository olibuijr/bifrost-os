# Backup (parity slice `backup`, round 75)

Scope: hourly snapshot backups with retention on an external disk, a Settings pane, browse and restore of a file
version from Files, and the `tmutil` command line.

## Done in code (host-testable, not yet on Midget)

| Area | BifrOSt path | State |
|---|---|---|
| Hourly schedule (`is_due`, `next_due`) | `core/src/snapshot_backup.rs` | Model with tests. Snapshots are due every hour. |
| Retention (24 h all, one per day for 30 days, one per week after) | `core/src/snapshot_backup.rs` `retention_plan` | Model with tests. Returns keep and drop lists. |
| Version lookup for restore (`version_at`) | `core/src/snapshot_backup.rs` | Model with tests. |
| Snapshot names (`YYYY-MM-DD-HHMMSS` UTC) | `core/src/snapshot_backup.rs` `stamp`, `parse_stamp` | Round-trip tested. |
| Exclusions read by the backup | `core/src/backup_exclude.rs` (`BackupExclusions`) | Existing model, reused by `tmutil`. |
| `tmutil status`, `startbackup`, `listbackups`, `restore [--at STAMP] SRC DEST`, `addexclusion`, `removeexclusion` | `cmd/src/tmutil.rs`, `cmd/src/bin/tmutil.rs` | Works against a file store. Unit tested. |

## Still a model (needs the backup writer and UI)

- **Backup writer (not built).** Nothing copies files into `snapshots/<stamp>/` yet, and nothing reads
  `start-request`. The store format is defined in `cmd/src/tmutil.rs` (`snapshots.txt`, `exclusions.txt`,
  `start-request`, `snapshots/<stamp>/<path>`). Retention is computed but not applied to disk.
- **Settings pane (not built).** Choose disk, back up automatically, exclusions, last and next backup. Status is
  available from `tmutil status`; no pane in `src/shell/pages.rs` yet.
- **Files: browse and restore a version (not built).** The lookup and copy exist in `tmutil restore --at`. No Files
  UI calls them.
- **Exclusion set:** `core/src/backup_exclusion.rs` is an older, separate exclusion model (`ExclusionSet`, its
  header says the backup service does not read it). It is a duplicate of `backup_exclude.rs`. Not merged in this
  round; the lead should pick one and delete the other.

## Checks still needed on Midget

- Settings pane layout and the disk-picker flow (after the pane exists).
- A real snapshot taken hourly, pruned by the retention plan, and one restored into Files.

## Reference evidence

No REA or `macos-ref` inspection was run in this round (time budget). The design uses only the rule names and the
slice brief. Before the Settings pane is built, read the shipped Time Machine settings extension with
`inspect_plist` and `decode_interface_builder` under `~/Projects/macos-ref/extract/ref27/`, and record the REA evidence
ids here.

## Name note

The command is named `tmutil` as the slice asks. That is an Apple command name; the brief bans Apple product names in
code. The guard in `tools/check-names.py` does not flag it today. The lead should decide whether to keep the name or
rename the binary (the module, bin and this doc would change together).
