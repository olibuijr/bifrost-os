# Backup, encryption and the backup destination

Code: `springboard/src/shell/backup.rs` (archive format, restore, destination), `backup_page.rs` (Settings > Backup and
Software Update), `timemachine.rs` (snapshot model, not wired).

## What is backed up

`backup::SOURCES`: the `Notes` and `Contacts` and `Calendar` folders, `reminders.tsv`, `settings.ini` and
`notifications.ini`, under the shell data directory. Folders listed in `backup-exclusions.txt` are left out.

## Archive format

- Version 1 (`SBBK1\n`): entry count, then for each entry the name, the data and a SHA-256 over both. The archive ends with
  a SHA-256 over all earlier bytes.
- Version 2 (`SBBK2\n`): the version 1 bytes sealed with XChaCha20-Poly1305. The 24-byte nonce is in the header, and the
  header is bound as associated data. The 256-bit key is in the shell keychain (app `bifrost.backup`, item
  `archive-key`), created on first use (`backup_key`).
- Version 1 archives stay readable without a key.
- A restore verifies every name and checksum before it writes anything. Each file is written through a temporary name and
  a rename.

## Restore

A restore needs the shell stopped. `stage` writes the decoded entries to `restore.pending`, and `apply_pending` applies
them at the next start, so the running shell never writes under itself.

## Destination

- `destination_root`: the chosen disk, or the data folder when none is chosen. Backups go to its `backups` folder.
- Settings > Backup offers the choice of disk. `backup_page.rs` shows the count of archives, the newest backup time and
  size, the last error and whether the newest archive verifies.

## Time Machine-style snapshots (`timemachine.rs`)

- A pure model: read-only btrfs snapshots of `@home` and `@system`, kept hourly for 24 hours, daily for 30 days, then
  weekly.
- Creating, pruning and mounting snapshots belong to privd (`backup.snapshot`, `backup.prune`). The module only builds
  the requests and the restore paths.
- The module is declared in `shell/mod.rs`, but nothing calls it yet. The Backups page does not offer snapshots.
