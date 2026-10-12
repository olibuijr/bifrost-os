# Wire 4: Files app (wire4/files)

Scope: `springboard/apps/files/` (Files 1.20.0). Two items from the wire3 gap list were wired: writes are coordinated
with other apps, and Get Info shows permissions, owner and group. Nothing was built or run in this round (the lead
builds once); every expected value below was worked out by hand.

## What is now wired

1. **Coordinated writes for paste, duplicate, compress and extract.** Before this round only Move to Trash took the
   coordination lock. Now `transfer_now` (Copy, Move, Paste, Duplicate), `compress_now` and `extract_now` hold the
   destination folder for writing for the whole batch, through the SDK `NativeFileCoordinator::coordinate_writing`.
   A reader in another coordinating app holds the same folder shared, so a file open there is not replaced under it.
   If the folder stays held for more than the five-second timeout, every item of the batch fails with
   "The item is in use by another app." and nothing is written.
   Code: `apps/files/src/coordinate.rs` (`folder_lock_path`, `writing_in`, `refused`), `apps/files/src/ops.rs`
   (`transfer_now`, `compress_now`, `extract_now`).
2. **Get Info: permissions.** A new row "Permissions" shows the item's mode as `ls -l` writes it, with the octal bits in
   brackets: `-rw-r--r-- (644)`, `drwxr-xr-x (755)`, `-rwsr-xr-x (4755)` for a set-user-ID file. The mode is read with
   the SDK `file_stat::stat` (typed `FileMode` from `bifrost_core::file_mode`).
   Code: `apps/files/src/info.rs` (`permission_rows`, `get_info`).
3. **Get Info: owner and group.** Rows "Owner" and "Group" show the account name, read from `/etc/passwd` and
   `/etc/group` (`account_name` parses the colon-separated tables), with the numeric id in brackets: `olibuijr (1000)`,
   `users (100)`. An id with no account shows the number alone, for example `1234`.

Pure helpers with host tests: `coordinate.rs` (`folder_lock_path` sits inside the folder, `refused` fails each item
with the sentence, `writing_in` runs under the folder lock) and `info.rs` (`permission_tests`: regular file,
directory, set-user-ID mode, id without an account, account lookup by id field only). Expected values are the ones
listed above, checked by hand against `FileMode`'s `Display` (the `ls -l` form) and the octal of `FilePermissions::bits`.

## How a user reaches it

- Coordinated writes: Copy, Cut and Paste, Duplicate, Compress and Extract work as before; nothing new to press. The
  difference shows only when another app holds the folder: the report reads "The item is in use by another app."
- Get Info: select an item and press Command-I (or choose Get Info from its context menu). The rows Permissions,
  Owner and Group follow Kind, Size, Where and the dates.

## How to verify on Midget

1. `bifrost pc publish`, then `bifrost pc midget ota` until `phase=accepted`.
2. Permissions: on Midget, `chmod 640 ~/Documents/note.txt`, then Get Info on it. Expected rows: `Permissions`
   `-rw-r----- (640)`, `Owner` `<user> (<uid>)`, `Group` `<group> (<gid>)`. For a folder, `chmod 755` shows
   `drwxr-xr-x (755)`.
3. Coordinated paste: create a folder `~/Share` with a file in it. In another terminal on Midget hold the folder for
   reading: `flock -s ~/Share -c 'sleep 60'` (the shared lock the coordinator's reads use). In Files, copy a file and
   Paste into `~/Share`. Expected: after about 5 s the notice reads "1 of 1 items could not be copied. <name>: The item
   is in use by another app." and no file appears in `~/Share`. When the `sleep` ends, the same paste succeeds.
4. Compress and Extract: select two files, choose Compress. Expected: `Archive.zip` appears in the folder. Choose Extract
   on it: a new folder with the two files appears beside it. Both still run without a lock holder.

## REA evidence

- `ev_b3e511348e3834b642481b38bee14d7823164494d785994edb0f6df33e8e6456`: rendered official page
  `https://developer.apple.com/documentation/system/filepermissions` (System, FilePermissions: owner, group and other
  read, write and execute permissions). `docc.py system/filepermissions` read the same symbol list.
- `ev_fddf330f85715b384d1de043b4206f3b9773ab5fe2f6e31548f35a65c296c84a`: rendered official page
  `https://developer.apple.com/documentation/foundation/nsfilecoordinator` (coordinated reading and writing among file
  presenters; a coordinated read or write runs in a background task when granted). `docc.py foundation/nsfilecoordinator`
  returned the same overview.
- Earlier coordination evidence still applies: `ev_71f9582976e270c31d2e3e76a27165a7f2b01952f7e71456072e9f73a1d5272d`
  (wire2 note, NSFileCoordinator overview).

## Remaining gaps

- **Extended attributes are not listed in Get Info.** Listing them needs `listxattr`, which std does not provide; the
  SDK `file_metadata` model is a pure diff and reads nothing. Needs a system-layer call before it can be shown.
- **The coordination lock is advisory and held through conflict prompts.** A process that does not coordinate is not
  blocked. The batch holds the destination folder while it waits for the user's answer to a name conflict, so another
  coordinating app waiting on that folder can time out during a long prompt. Moving the lock to each item's write would
  need a change in the shared batch module outside this app.
- **Move does not lock the source folder.** Only the destination is held; a source item read by another app during a
  cross-folder move is not protected. Locking both folders would nest two locks, which is avoided on purpose.
- **Presenters are not registered for the open folder.** Other apps' changes reach Files through the folder watcher,
  not through `NSFilePresenter`-style callbacks.
- **Archives are ZIP only.** Compress and Extract use the Files ZIP engine (`springboard::files::zip`) directly, the same
  engine the SDK `archive` model wraps. The Apple Archive container format, its headers, ownership and extended
  attributes are not built. Extract works on the focused single `.zip` only.
- **Owner names come from `/etc/passwd` and `/etc/group` only.** Directory services (LDAP, NSS) are not consulted.
- Not verified: no build or test run (the lead builds once). The compliance and snapshot tests do not cover the new
  Get Info rows; the lead should check `apps/files/tests/` if a snapshot shows the Get Info panel.

## Matrix rows

| Slug | Feature (as in `docs/parity-matrix/data/<slug>.json`) | BifrOSt path |
|---|---|---|
| foundation-files-persistence | File coordination and presenters | springboard/apps/files/src/coordinate.rs (writing_in, refused); springboard/apps/files/src/ops.rs (transfer_now, compress_now, extract_now) |
| system | Advisory directory locks for coordinated reads and writes | springboard/apps/files/src/coordinate.rs (writing_in); springboard/apps/files/src/ops.rs (transfer_now) |
| system | File permissions and modes | springboard/apps/files/src/info.rs (permission_rows, Permissions row) |
| system | File status (stat, file size, link count, owner, inode, timestamps) | springboard/apps/files/src/info.rs (permission_rows, Owner and Group rows; account_name) |
| foundation-files-persistence | File System (create, read, write, examine files) | springboard/apps/files/src/info.rs (get_info reads the item's status) |

Rows are not promoted to `parity` here: each needs the Midget checks above first (`bifrost pc parity` update after Midget
passes). The Extended attributes part of "File status" and the Apple Archive rows are not changed by this round.
