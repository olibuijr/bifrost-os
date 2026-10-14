# System: typed file paths (sdk-rest-06)

Mirrors the component-wise path type of the System framework: a path made of a root and parts, where a part is a name,
`.` or `..`.

## What is wired

- `springboard/sdk/src/typed_path.rs` (`bifrost_app::typed_path`): `TypedPath`, `Part`, `PathError`.
  - `TypedPath::parse` and `FromStr` build a path from text. Empty text, a NUL byte and separators inside a name are
    refused with `PathError`.
  - `push`, `remove_last`, `file_name`, `parts`, `is_absolute` work on the components.
  - `lexically_normalized` resolves `.` and `..` on the text without touching the file system.
  - `to_path_buf` hands the same text to the language path type, which the rest of the SDK still takes.
- Tests: `springboard/sdk/tests/parity_sdk-rest-06.rs` (headless, in memory) and the module's own tests.

## What is not here

- No file system calls. Opening, statting and reading remain the job of `bifrost_app::file_manager` and the
  file coordination module.
- No platform byte string with a length prefix. The language path type stays the storage type, so this module adds
  checks and component views, not a second representation.
- Needs no lab machine: every behaviour is pure text handling and is tested on the host.
