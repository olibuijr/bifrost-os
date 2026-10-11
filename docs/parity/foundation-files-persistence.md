# Foundation (Files and Data Persistence): parity work

Official reference: https://developer.apple.com/documentation/foundation (Files and Data Persistence: File System,
Archives and Serialization, Settings, Spotlight, iCloud, backup).

REA evidence of the rendered official landing page: `ev_60ea0b0305ae4fc0c01f500535d342081d1a51aee160b0c23afd18e3fc5c328e`.
Official API shape came from `tools/parity/docc.py` on `foundation/filemanager` and `foundation/nskeyedarchiver`.

## What was built

1. **File System, FileManager-style item API.** `springboard/sdk/src/file_manager.rs`, type `NativeFileManager`
   (std only, no unsafe). Mirrors these official methods:
   - `fileExists(atPath:isDirectory:)` as `item_kind` and `file_exists`
   - `attributesOfItem(atPath:)` as `attributes_of_item` (kind, size, modified time)
   - `contentsOfDirectory(atPath:)` as `contents_of_directory`
   - `subpathsOfDirectory(atPath:)` as `subpaths_of_directory` (deep, relative, depth first)
   - `createDirectory(atPath:withIntermediateDirectories:)` as `create_directory`
   - `copyItem(at:to:)` as `copy_item`, `moveItem(at:to:)` as `move_item`, `removeItem(at:)` as `remove_item`

   Safety rules follow the official behaviour: an existing destination is refused, links are never followed by
   copy, move or remove, and a cross-device move copies then removes, cleaning up a partial copy on failure.

2. **Spotlight-style metadata queries.** `springboard/ui/src/metaquery.rs` (`bifrost_ui::metaquery`), a pure model.
   It mirrors the metadata query predicates (display name, extension, directory flag, size, modification time)
   combined with all, any and not, plus sort descriptors and a result limit. The caller supplies the records.

## Tests

Unit tests sit in each module: 7 in `file_manager.rs` (tree copy, refused overwrites, rename, relative sorted
enumeration, link handling, attributes, intermediate directories) and 6 in `metaquery.rs` (name matching, extension
rules, boolean combinations and empty lists, size and time bounds, sort and limit, empty input). No build or test was run
in this stage; another stage builds and runs them.

## Remaining gaps

- FileWrapper (bundles), bookmarks and security-scoped URLs. User files still go through the portal grants.
- Symbolic link creation, extended attributes and copy progress.
- Keyed archiver (`NSKeyedArchiver` format) and binary property lists.
- A file-system-wide metadata query with live updates. The current model only filters records the caller gathered.
- iCloud documents and key-value store (Apple service, out of scope).
- Backup exclusion and on-disk purgeable data.
- File versions.

## Matrix

Row status changes are in `docs/parity-matrix/data/foundation-files-persistence.json`: "File System" stays `partial`
(the FileManager-style API is in place, the wider surface is not), and "Spotlight: metadata queries" moves from `gap`
to `partial`.
