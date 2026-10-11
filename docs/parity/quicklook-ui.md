# Quick Look UI: parity work

Official reference: https://developer.apple.com/documentation/quicklookui (macOS 12.0 and later for the collection; the
panel and item protocols date from macOS 10.6).

Status after this round: `partial` (unchanged framework status). The matrix row data is in
`docs/parity-matrix/data/quicklook-ui.json`.

## What was built

All code is in `springboard/sdk/src/peek.rs` (extends the existing module; no new module, no wire change).

1. **Data source protocol** (`QLPreviewPanelDataSource`, `numberOfPreviewItems(in:)`, `previewPanel(_:previewItemAt:)`).
   - `PreviewDataSource` trait: `number_of_items()` and `item_at(index)`. `Vec<PreviewItem>` implements it.
   - `PreviewPanel::load_from(&dyn PreviewDataSource)` pulls at most `MAX_ITEMS` (16) items and clamps the current index.
   - `PreviewPanel::reload_data_from(source, ctx)` pulls, then re-sends the list while the panel is visible.
2. **Item display state** (`QLPreviewItem.previewItemDisplayState`).
   - `ItemState` enum: `Loading`, `Ready` (default), `Failed`.
   - `PreviewPanel` keeps one state per item. `set_items` resets every state to `Ready`.
   - `display_state(i)` returns `None` past the end. `set_display_state(i, state)` returns `false` and changes nothing past the end.
3. **Refresh** (`refreshCurrentPreviewItem()`): `refresh_current_item(ctx)`. The wire has no per-item message, so it
   re-sends the whole list while visible.

Tests (in the same module): ordered pull and the 16-item cap, index clamping on pull, `Vec` as a data source, display
state defaults and reset, and refusal of out-of-range state writes. Build and test are not run in this round.

## Official API names mirrored

| Official | BifrOSt |
|---|---|
| `QLPreviewPanelDataSource` | `PreviewDataSource` |
| `numberOfPreviewItems(in:)` | `number_of_items` |
| `previewPanel(_:previewItemAt:)` | `item_at` |
| `QLPreviewItem.previewItemDisplayState` | `ItemState`, `display_state`, `set_display_state` |
| `refreshCurrentPreviewItem()` | `refresh_current_item` |
| `reloadData()` (with a data source) | `reload_data_from` |

## REA evidence

- `ev_1d4e873e87bea38bea30ec31b6971f7d4b059ac6e26e01c45a2e2b8a95a46597`: rendered official landing page
  (`tools/parity/rea-page.sh https://developer.apple.com/documentation/quicklookui`), this round.
- `ev_70ff98192db4e16d1b1cce2de3a6bd86a4cbb2cf6ead2a6b1d6fee5673d6864b`: the earlier matrix row evidence.
- Symbol lists for `QLPreviewPanel`, `QLPreviewItem` and `QLPreviewPanelDataSource` were read with `tools/parity/docc.py`.

## Remaining gaps

- The shell does not draw the panel. `ToShell::Preview` and `PreviewClose` are decoded and ignored.
- Display state stays in the SDK. A `Loading` or `Failed` item cannot show its state on screen.
- No delegate protocol (`QLPreviewPanelDelegate`) and no asynchronous loading block (`QLPreviewItemLoadingBlock`).
  `item_at` is synchronous.
- No preview extension points (`QLPreviewingController`, `QLPreviewProvider`, `QLFilePreviewRequest`, `QLPreviewReply`).
- No embeddable preview view (`QLPreviewView`).
- No built-in renderers for thumbnails, no thumbnail update handlers, no multi-representation requests.
- Panel-level state (`displayState`, full-screen mode, `currentController`, `sharedPreviewPanelExists`) is missing.
