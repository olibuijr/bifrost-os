# Wire: the `pasteboard` system service (typed items, named pasteboards, change count)

Matrix rows this moves (`docs/parity-matrix/data/`):

- `application-services.json`: "Pasteboard (NSPasteboard)" (partial)
- `core-transferable.json`: "Copy and paste of typed items" (partial)

## What is now wired

Before this round the pasteboard had one general board, typed items (`NSPasteboard` in the SDK, `ToShell::PasteboardSet`)
and no named boards, no items with several representations, and no change notification to other apps. The `pasteboard`
service adds all three over the named service channel. The wire protocol is unchanged.

- **Shell** `springboard/src/shell/services/pasteboard_types.rs`: the `Pasteboards` service (`SystemService`, name
  `pasteboard`), registered in `services::all()`.
  - Items are `Item { reps }`: several representations of one copy (plain text, link, file URL, PNG, rich text).
  - Each board has a change count. A write (`clear` plus `write`) is one change.
  - `tick` pushes one `changed` event per board whose count moved, to every running app, with `{board, change_count}`.
  - The `general` board is the shell clipboard. Its reads come from `shell::pasteboard::snapshot`, and its writes go through
    `shell::pasteboard::set`, the plain clipboard (`clipboard::set_text`) and `offer_wayland`. Changes made over the older
    protocol also show up as `changed` events for `general`.
  - Reuses the UTType model `bifrost_ui::uttype` for `can_read` (type conformance) and `shell::pasteboard::mime_for` for the MIME
    type of each representation.
- **Methods** (JSON params, `board` defaults to `general`): `boards`, `unique`, `release {board}`, `write {board, items}`,
  `clear {board}`, `change_count {board}`, `read {board}`, `types {board}`, `string {board, uti}` (text of every item that
  holds the type, concatenated, as Apple's `string(forType:)`), `data {board, uti}` (first item, hex), `can_read {board,
  types}`.
- **Wire shape of an item:** `{"reps": [{"uti": "public.utf8-plain-text", "text": "hi"}, {"uti": "public.png", "hex": "89504e47"}]}`.
  Data that is valid UTF-8 travels as `text`; other bytes as lowercase hex. Limits: 16 items a write, 16 representations an
  item, 16 named boards.
- **SDK** `springboard/sdk/src/pasteboard_types.rs` (`bifrost_app::pasteboard_types`): `Board` (`general`, `named`, `write`,
  `clear`, `change_count`, `read`, `types`, `string`, `data`, `can_read`, `release`), `Item::with`, `unique`, `boards`, and
  decoders for the replies (`decode_change_count`, `decode_read`, `decode_types`, `decode_string`, `decode_data`,
  `decode_bool`, `decode_names`, `decode_board_name`) and for the event (`decode_change`). Calls return the request id; the
  reply arrives in `App::service_reply`. The SDK reuses the shell's `to_hex` and `from_hex`.

## How a user reaches it

The service is for apps. A user reaches it through an app that calls it: a copy in one app (`write` to a named board, or to
`general`) shows up in another app's `read`, and the other app gets a `changed` event with the new count and can refresh.
No Settings or menu entry exists yet.

## How to verify on the test PC (Midget)

Midget only; the lab VM is not used for this round.

1. Build and publish as usual: `bifrost pc publish`, then `bifrost pc midget ota` until `phase=accepted`.
2. Check the service is registered: call the `system` service's `services` method from an app, or read the shell log. The
   list must include `pasteboard`.
3. Write and read a named board from one app, with a second app open:
   `pasteboard write {"board":"demo","items":[{"reps":[{"uti":"public.utf8-plain-text","text":"hi"}]}]}`, then
   `read {"board":"demo"}`. Expect `change_count` 1 and the text back. The second app must receive
   `changed {"board":"demo","change_count":1}`.
4. Write `general` (`write` with no board) and check the text reaches the plain clipboard (paste it in a text field).
5. Screenshot the result with `bifrost pc midget screen`. No UI changes in this round, so the check is functional only.

## REA evidence

- `ev_a56d232a88893f8c75a787655b47c8fcee754bfbfca75a880b0ce1fff09c3d10`: rendered page
  `https://developer.apple.com/documentation/appkit/nspasteboard` (pasteboard server shared by all apps, multiple items,
  `general`, `clearContents`, `setData(_:forType:)`, `changeCount`, `pasteboardItems`, `name` and `init(name:)`).
- Official documentation read with `tools/parity/docc.py` (paths `appkit/nspasteboard`, `appkit/nspasteboarditem`,
  `appkit/nspasteboard/changecount`): creating with a name, `withUniqueName()`, `releaseGlobally()`, `clearContents()`,
  `writeObjects(_:)`, `setData(_:forType:)`, `string(forType:)` (concatenation over items), `data(forType:)` (first item),
  `availableType(from:)`, `canReadItem(withDataConformingToTypes:)`, `changeCount`.

## Gaps

- Access control: no pasteboard access behaviour (the official `accessBehavior` prompt). Any app can read and write any named board.
- Named boards are kept in memory and lost on a shell restart; they are not persisted.
- The general board is flat (one representation per item). The shell clipboard state does not keep item grouping.
- A write to `general` through this service does not re-broadcast the `ToApp::Pasteboard` mirror, so an app that reads
  `pasteboard::NativePasteboard` (the synchronous path) stays stale until its next copy. Use the service's `read`.
- Drag and drop of typed items is not covered by this service (the drag path is separate).
- No `NSPasteboardWriting` or `NSPasteboardReading` object protocol. Items are raw representations.
- Property lists (`setPropertyList`) are not wired.
- Detection (`detectedPatterns`, `detectedValues`) on pasteboard items is not wired; the `datadetection` model in
  `springboard/ui/src/detect.rs` could run over `text` representations.
- Not run on the device yet: the host tests in the two new files are written but the build is left to the lead, and the
  Midget steps above are the plan, not a result.
