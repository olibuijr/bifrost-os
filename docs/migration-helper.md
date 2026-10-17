# Migration helper

Code: `springboard/src/shell/migration.rs`. Uses the backup restore in `shell/backup.rs`.

Brings a user's data from another disk, or from a backup archive, into this machine's data folder.

## Sources

- An archive made by `backup::create`.
- A data folder on another disk.

Contents are listed as categories: one per shell data set, and one per app's data folder (`appdata/<app id>/`). Only the
categories the person picked are staged. Paths are checked, so an entry outside its category is refused.

## Applying

- The staged copy goes through `backup::write_entries`. It is applied at the next start by `backup::apply_pending`, as a
  backup restore is, so the running shell never writes under itself.
- An archive is opened with the destination's archive key. An archive from another machine is readable only when that
  machine's keychain came along, for example on a reinstall that keeps the data folder.
