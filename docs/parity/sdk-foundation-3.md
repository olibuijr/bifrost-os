# sdk-foundation-3: Foundation parity, round 3

Slice: foundation-* parts of the parity matrix (Core Foundation, Foundation Fundamentals, Files and Data
Persistence, App Support). Rows were read with `python3 tools/bifrost.py pc parity show "foundation"`. Matrix JSON was
not edited; the lead closes rows.

Evidence: the reference names come from the parity index rows, not from REA extracts. The macOS 27 reference
extracts under `~/Projects/macos-ref/extract/` were not measured in this round (no REA evidence used).

## Done this round

### Property lists: binary read (bplist00)
- Rows: Core Foundation "Property lists and Codable-style encoding (CFPropertyList)"; Files and Data Persistence
  "Archives and Serialization: JSON and property lists" (was "no binary plist").
- API: `bifrost_core::plist::parse_bytes` (used by `plutil`) now reads binary property lists through the new
  `bifrost_core::plist_binary::parse`. SDK: `bifrost_app::coding::PropertyListDecoder::decode_bytes` decodes binary or
  text bytes into any `Decodable`.
- Path: `springboard/core/src/plist_binary.rs` (reader, tests with hand-built fixtures), `core/src/plist.rs`
  (`parse_bytes` dispatch; the old "not read yet" test now asserts a parse error), `sdk/src/coding.rs`
  (`decode_bytes`, `plist_value`, `binary_tests`).
- Mapping: null reads as an empty string (as the XML writer writes it); binary dates become UTC text and decode like
  XML dates; dictionaries keep stored order; UIDs are refused; data values are refused by Codable (no `Value` form
  for data yet); depth capped at 512 so cyclic references fail cleanly.
- Still open: writing binary property lists (`plutil -convert binary1` is refused); UID values; the Codable value tree
  has no data case.
- Midget check: `plutil -lint` on a binary `.plist` from a shipped app bundle (host tests only so far).

### Notification queue
- Row: Foundation "Notifications (NotificationCenter, DistributedNotificationCenter, NotificationQueue)".
- API: `bifrost_app::notification_queue::NotificationQueue` with `enqueue(n, PostingStyle, Coalescing)`,
  `remove(name, object)`, `take_pending()`, `deliver_pending()`. Coalescing on name, sender or both; `PostingStyle::Now`
  is handed back for an immediate post.
- Path: `springboard/sdk/src/notification_queue.rs` (pure model with unit tests), declared in `sdk/src/lib.rs`.
- Gap: `WhenIdle` delivers on the next main-queue drain, not on real idle (no idle signal yet). Not wired into any app
  or the shell (no caller existed to migrate).
- Midget check: none needed until an app adopts it.

## Not changed (checked, rows still Partial)
- Number and date formatters: `springboard/sdk/src/locale.rs` already has `NumberFormatter`, `DateFormatter`,
  `relative_date_time`, `byte_count`, `measurement`. The matrix note ("fixed helper formats only") is stale, but
  no row was closed on it this round.
- URL sessions: `springboard/sdk/src/url.rs` covers sessions, tasks and cache. No new work this round.
- File manager: `springboard/sdk/src/file_manager.rs` has copy, move, remove, create directory, attributes and
  enumerators. The "FileManager-style SDK API" gap in the Files row is stale; not closed here because `FileHandle` and
  `FileWrapper` are still missing.
- Undo manager: `springboard/ui/src/undo.rs` already shared by notes, reminders and rich text.
