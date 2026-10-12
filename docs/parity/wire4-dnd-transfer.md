# Wire 4: drag and drop, clipboard and transfer types

Owner: the wire4 round (branch `wire4/dnd-transfer`). Builds on `docs/parity/wire-svc-pasteboard.md` (the `pasteboard`
service), `docs/parity/wire3-share-compose.md`, and the typed drag and drop path in `docs/bifrost-ui.md`
("Typed drag and drop"). The wire protocol (`springboard/proto`) is unchanged.

## What is now wired

- **Transferable declarations reach copy, paste and drag.** `bifrost_app::transfer` (`springboard/sdk/src/transfer.rs`) was a
  pure model with no caller. It now has bridges for the three transfer paths:
  - `transfer::pasteboard_item(reps, value)`: one pasteboard item with a representation for every type another app may see
    (own-process types never leave the process).
  - `transfer::from_pasteboard_item(reps, item)`: rebuilds the value from the first importable type the item holds
    (`best_import`, in the model's preference order).
  - `transfer::drag_item(reps, value)` and `transfer::begin_drag(ctx, allowed, value, reps, preview)`: a drag item offered as
    every visible UTType, carrying the text (or, failing that, the link) of the value as its payload. `None` when nothing
    can be carried, so no session starts.
  - `transfer::drop_values(reps, items)`: on the destination side, rebuilds the values of the dropped text and link items;
    file items and items the model cannot import are skipped.
- **Type negotiation on named pasteboards.** The `pasteboard` service (`springboard/src/shell/services/pasteboard_types.rs`)
  gains a `best {board, types}` method: the first preferred type the board holds, by exact match (as Apple's
  `availableType(from:)`), or `null`. The model is `Board::best_type`; the SDK client is `pasteboard_types::Board::best` with
  `decode_best`.

## How a user reaches it

Drag and drop and the clipboard are used by apps, not by a Settings page, so the user reaches this through an app that
calls it. Typed values now travel with their declared representations: a value copied to a named board carries both
its plain text and its other types, and a drag between two apps carries the text of the value with the UTTypes it
declares. Nothing is added to the menus or the Settings app in this round.

## How to verify on the test PC (Midget)

Midget only; the lab VM is not used.

1. `bifrost pc publish`, then `bifrost pc midget ota` until `phase=accepted`. Then `bifrost pc midget state` to confirm the
   shell is up.
2. Copy and paste through a named board: from an app that uses `transfer::pasteboard_item` (or `pasteboard_types::Board`
   `write`), write a board with a text and an HTML representation. Then read it with `Board::best` over
   `["public.png", "public.html"]`; expect `public.html`. A preference list with no held type gives `null`.
3. Drag between two apps: from an app that calls `transfer::begin_drag` with a value whose representation is text, drag over
   a second app that registered `public.text` (`dnd::register_types`). The destination's `App::drag` receives `Entered`
   with the item's types; on release, `drop_values` on the `Perform` items returns the text.
4. Screenshot with `bifrost pc midget screen` as evidence. No UI changed in this round, so the check is functional.

## REA evidence

- `ev_97b37334ae46ae53a24220cf253f6b8827916c0822cb148d61f9f338db41b1c1`: rendered page
  `https://developer.apple.com/documentation/coretransferable` (Transferable, drag and drop and copy and paste as the
  system interactions, `draggable(_:)`, `dropDestination`, the data, file, proxy and visibility groups).
- Official symbol text read with `tools/parity/docc.py`: `coretransferable`, `coretransferable/transferrepresentation`
  (visibility, `exportingCondition`), `coretransferable/transferrepresentationvisibility`,
  `coretransferable/filerepresentation` (`init(contentType:...)`, `init(exportedContentType:...)`),
  `appkit/nsdraggingsession` (`draggingPasteboard`).

## Gaps

- **File promises are not wired.** Dragging a file out of an app to Files needs a promise that the destination fulfils at
  drop time. The drag payload is `Payload::File` with an existing document grant, and the wire protocol cannot ask the
  source for the bytes at drop. No promise model exists in `springboard/ui/src`, so none is wired.
- **File representations have no sender and receiver roles** (`SentTransferredFile`, `ReceivedTransferredFile`). Files still
  move as document grants.
- **Images in drags** carry no image payload (`public.image` is a type match only), as before.
- **Codable types** have no bridge to the pasteboard or drag (`CodableRepresentation` is not in the model).
- **The general clipboard** stays flat (one representation per item). A copy through `transfer::pasteboard_item` to `general`
  keeps the item grouping only on named boards.
- **Drop targets are not declarative.** `drop_values` is a helper; the view kit still has no drop modifier.
- **Visibility** still separates the owning process from everyone else; there is no team level.
- Host tests were written by hand and not run (the lead builds once). Midget steps above are the plan, not a result.

## Matrix rows

The matrix data is not edited in this round (the lead publishes it).

| slug | feature (as in the matrix json) | BifrOSt path |
|---|---|---|
| core-transferable | Transferable protocol and TransferRepresentation builder | springboard/sdk/src/transfer.rs (`pasteboard_item`, `from_pasteboard_item`, `drag_item`, `begin_drag`, `drop_values`) |
| core-transferable | Drag and drop of typed items | springboard/sdk/src/transfer.rs (`begin_drag`, `drop_values`); springboard/sdk/src/dnd.rs |
| core-transferable | Copy and paste of typed items | springboard/sdk/src/transfer.rs (`pasteboard_item`, `from_pasteboard_item`); springboard/sdk/src/pasteboard_types.rs (`Board::best`); springboard/src/shell/services/pasteboard_types.rs (`best`) |
| core-transferable | Proxy representation and transfer visibility | springboard/sdk/src/transfer.rs (`pasteboard_item` hides own-process types) |
| core-transferable | File transfer (file URL representations) | not wired (see gaps: file promises) |
| application-services | Pasteboard (NSPasteboard) | springboard/sdk/src/pasteboard_types.rs (`Board::best`); springboard/src/shell/services/pasteboard_types.rs |

Expected status after Midget verification: the first three rows move to `partial` with the wired paths listed; the proxy and
visibility row can move to `partial` on the same path; file transfer stays `partial`/gap until file promises exist.
