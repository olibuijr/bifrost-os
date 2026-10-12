# System: typed file modes, errno and file status (wave 4)

Branch `parity/w4-system`. Code only; not built or run (no builds in this wave).

## What was built

- `springboard/core/src/file_mode.rs` (`bifrost_core::file_mode`): `FileMode` (a full mode word with
  `parse_octal`, `file_type`, `permissions`, `masked` for a umask, and an `ls -l` style `Display`),
  `FileType` and `FilePermissions` (a typed permission set with the owner, group, other, set-user-ID,
  set-group-ID and sticky bits, combined with `|` and tested with `contains`). `ModeError` reports why
  octal text is refused.
- `springboard/core/src/errno.rs` (`bifrost_core::errno`): `Errno`, a typed system error number with
  named constants for 25 common Linux values, `name`, `message`, `to_io_kind` (the closest
  `std::io::ErrorKind`) and `from_io` to read the number back from an `io::Error`. `Display` prints
  `No such file or directory (ENOENT)`.
- `springboard/sdk/src/file_stat.rs` (`bifrost_app::file_stat`): `FileStat` (device, inode, typed mode,
  link count, owner, size, allocated blocks, block size, and access, modify and change `Timestamp`s),
  built from `std::fs::Metadata` through `MetadataExt`, with `stat` (follows a final link) and `lstat`
  (describes the link). No `unsafe`, no new crate, no system call beyond the std metadata calls.

Module registrations: one `pub mod` line each in `springboard/core/src/lib.rs` and
`springboard/sdk/src/lib.rs`.

Not built: the stat flags, file generation number, typed open options and positional read/write remain
open (see the table).

## Official API names mirrored

- System > File Permissions (`FilePermissions`): https://developer.apple.com/documentation/system/filepermissions
  Owner, group and other permission members, the special permissions (set user ID, set group ID, save
  text), and the raw value. Mirrored as `FilePermissions` constants and `from_bits` / `bits`.
- System > Errno: https://developer.apple.com/documentation/system/errno
  The file, path, device and network error members. Mirrored as `Errno` constants with the platform
  names; the Swift member names are not used in code.
- System > Stat: https://developer.apple.com/documentation/system/stat
  Instance properties `deviceID`, `inode`, `mode`, `linkCount`, `userID`, `groupID`, `size`,
  `blocksAllocated`, `preferredIOBlockSize`, `st_atim`, `st_mtim`, `st_ctim`, `type`, `permissions`.
  Initializers with a path and `followTargetSymlink` map to `stat` and `lstat`. Stat flags and
  `generationNumber` are not mirrored.

## REA evidence

- FilePermissions page: `ev_5158341a5359a847f303da863cd8923cd687b1824c622525e095330e24a0f113`
- Errno page: `ev_07f2fecf234f61a03e25f626642b239e6324d2f997919d231f0d40ced84388a6`
- Stat page: `ev_fd54a000e7728899a0255ceeb0f76e2ae3b998d420e41da91697167127b987c6`

DocC (`python3 -I tools/parity/docc.py system/filepermissions`, `system/errno`, `system/stat`) gave the
member lists above.

## Matrix rows closed

These rows move to `partial` in the matrix (the lead applies the status change centrally).

| Framework slug | Feature (exactly as in `docs/parity-matrix/data/system.json`) | New status | BifrOSt path |
|---|---|---|---|
| system | File permissions and modes | partial | springboard/core/src/file_mode.rs (FilePermissions, FileMode); springboard/sdk/src/file_stat.rs |
| system | Error numbers (Errno) | partial | springboard/core/src/errno.rs (Errno) |
| system | File status (stat, file size, link count, owner, inode, timestamps) | partial | springboard/sdk/src/file_stat.rs (FileStat, stat, lstat) |
| system | Typed system-call results and option sets (FileFlags, FileMode bits, open options) | partial | springboard/core/src/file_mode.rs (FilePermissions option set, FileMode bits); open options and file flags not done |

The row for typed file ownership (UserID, GroupID, DeviceID, Inode) is covered only as `u32`/`u64`
fields in `FileStat`; it was not closed.

## Remaining in System after this wave

Typed open options and file flags, positional read and write, pipe creation, volume capacity (needs a
system call that uses `unsafe`, so it was left out), and typed path components.

## Checks run

- `python3 tools/check-names.py`: exit 0.
- Tests written by hand in each new module (none run; no builds in this wave).
