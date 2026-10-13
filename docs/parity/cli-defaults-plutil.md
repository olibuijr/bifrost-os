# CLI: defaults, plutil (slice cli-defaults-plutil)

Reference: the shipped man pages `defaults.1` and `plutil.1` under `~/Projects/macos-ref/extract/ref27/usr/share/man/man1/`
(read with `man -l`). No REA evidence was used, and the extract's app bundles were not measured for this slice. Behaviour
comes from the man pages, the existing per-app defaults file the SDK writes, and the shell's app data layout.

Matrix entries (not edited here, the lead closes them): `path:/usr/bin/defaults` (cli/d-f.json) and
`path:/usr/bin/plutil` (cli/p-r.json).

## Code

- `springboard/core/src/plist.rs`: the property-list model (`Plist`) with XML and ASCII readers, XML, ASCII, print and
  JSON writers, key-path get, replace, insert, append and remove, and base64 and hex codecs. Binary input is refused by name.
- `springboard/core/src/defaults_file.rs`: the defaults file codec (`kind<TAB>key<TAB>value`), moved here from the SDK. It
  gained a `p` kind so arrays and dictionaries can be stored.
- `springboard/sdk/src/defaults.rs`: `UserDefaults` now uses the shared codec. `Value` is re-exported from core and gained
  a `Plist` variant. New `plist(key)` and `set_plist(key, value)` read and write arrays and dictionaries. Removed: the
  private `escape`, `unescape`, `serialise`, `parse` and `unhex` helpers.
- `springboard/cmd/src/defaults.rs`, binary `springboard/cmd/src/bin/defaults.rs`.
- `springboard/cmd/src/plutil.rs`, binary `springboard/cmd/src/bin/plutil.rs`.
- `springboard/core/src/lib.rs` and `springboard/cmd/src/lib.rs`: two `pub mod` lines each.

## Store

`defaults` works on the same files the SDK writes: `<root>/appdata/<domain>/defaults.txt`. `<root>` is
`SPRINGBOARD_DATA` when set, else `$XDG_DATA_HOME/springboard`, else `~/.local/share/springboard`. The OS shell's store
is `/var/lib/springboard`, so set `SPRINGBOARD_DATA=/var/lib/springboard` to read the system store. The global domain is
named `GlobalDomain` (`-g` and `-globalDomain` name it). No app reads it yet, which matches the SDK.

## defaults

| Man page form | Here | Path |
|---|---|---|
| `read`, `read domain`, `read domain key` | done. Strings raw, booleans 1 or 0, data as hex, collections in the ASCII form. Missing pairs use the man page's message | `defaults.rs` `run` |
| `read-type domain key` | done: `Type is string`, `boolean`, `integer`, `float`, `data`, `date`, `array`, `dictionary` | `type_word` |
| `write domain key value` (no flag) | done. A value that reads as a property list is stored as one, else as text | `parse_write` |
| `write domain 'plist'` | done: the whole domain is replaced by the dictionary | `Command::WriteAll` |
| `-string -int -float -bool -data -date` | done. `-bool` takes TRUE, FALSE, YES or NO | `typed_set` |
| `-array`, `-array-add`, `-dict`, `-dict-add` | done: arrays and dictionaries are stored as the `p` kind | `Set`, `apply` |
| `rename domain old new` | done | `run` |
| `delete domain [key]`, `delete-all` | done. Without a domain it is refused, so nothing is wiped by accident | `run` |
| `import domain path \| -`, `export domain path \| -` | done, as XML text. Export writes XML, not a binary plist | `run` |
| `domains` | done: comma-separated, sorted | `run` |
| `find word` | done: matches domain names, keys and values, case-insensitive. Output is one line per match, not the man page's summary | `run` |
| `-currentHost`, `-host`, `-verbose` | accepted. There is one host | `parse` |
| `-app`, `-container`, plist-file domains | refused with "not supported yet" | `parse`, `domain_arg` |

Gaps: the `-container` and `-app` lookups need an app bundle and container store that the shell does not expose to the
command line. Binary plist export is not written yet.

## plutil

| Man page form | Here | Path |
|---|---|---|
| `-help`, `-lint` (default), `-p` | done. `-lint` prints `file: OK`, `-s` silences it. `-p` prints the arrow form with sorted keys | `process` |
| `-convert xml1`, `-convert json` (with `-r`) | done. `-o path` (or `-` for stdout) and `-e ext` pick the output | `process`, `target_for` |
| `-convert binary1`, `-create binary1` | refused with a message: binary input and output are not written yet | `render` |
| `-convert swift`, `-convert objc` | refused: not supported yet | `render` |
| `-insert keypath -type [value] [-append]` | done for all types, including `-xml` and `-json` fragments. Dictionary keys must be new | `insert`, `value_for` |
| `-replace keypath -type value` | done | `replace` |
| `-remove keypath` | done | `remove` |
| `-extract keypath xml1 \| json \| raw` (`-n`, `-expect`) | done. Raw follows the man page's table | `raw_text` |
| `-type keypath [-expect type]` | done | `process` |
| `-create xml1 \| json` | done | `process` |
| `--`, `-`, `-s` | done | `parse` |

Edits and conversions write XML text back to the file (or to `-o`). Apple's tool keeps the input's format; the input here
is XML or ASCII, and the output is XML unless `-convert json` was asked for.

Gaps: binary property lists (both directions), and `swift` and `objc` output.

## Tests

Unit tests in `core/src/plist.rs`, `core/src/defaults_file.rs`, `cmd/src/defaults.rs` and `cmd/src/plutil.rs`, plus
the SDK tests in `sdk/src/defaults.rs`, which still pass through the shared codec. The lead builds and runs them.

## Still needs Midget checking

- An app's `UserDefaults` written by the SDK, then read with `defaults read <domain> <key>` from a user session.
- `defaults write` followed by a running app that reads the key, to confirm the value survives a restart.
- `plutil -p` on a real app bundle's `Info.plist` once one is extracted under `~/Projects/macos-ref/extract/ref27/`.
