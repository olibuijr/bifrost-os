# Core Transferable parity

Framework: Core Transferable (macOS 13 and later). Status: partial. Branch: `parity/team-k-core-transferable`, not yet verified on Midget.

## What is built

- `springboard/sdk/src/transfer.rs` is the `Transferable` model. A model type declares an ordered list of representations
  (`Representations`): `data`, `codable`, `proxy`, `file` and `proxy_of`. Each has a UTType id, a visibility
  (`All`, `Team`, `OwnProcess`) and, when readable, an importer.
- `proxy_of` forwards another `Transferable`'s representations: bytes and file. Visibility is narrowed to the stricter of the
  two, and a forwarded type with no importer stays export-only.
- The pasteboard (`pasteboard.rs`), typed drag and drop (`dnd.rs`, `transfer_drag.rs`), the item provider (`item_provider.rs`)
  and the share picker (`share.rs`) consume the same declaration.
- Unit tests are in `transfer.rs` and `springboard/sdk/tests/transferable.rs`. Host tests only; nothing was run in this branch.

## Remaining gaps

- No app adopts `Transferable` yet; the declarations exist only in the SDK and tests.
- No derive-style `CodableRepresentation`: encoders are passed as functions.
- A file representation travels as bytes on a pasteboard item; the shell publishes one rich type to the Wayland selection
  (`springboard/src/shell/pasteboard.rs`) and has no file promise.
