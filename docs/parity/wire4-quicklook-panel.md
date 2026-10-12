# Wire 4: Quick Look preview panel (services `quick-look`, Files Space)

Branch `wire4/quicklook-panel`. Builds on `wire-svc-quicklook.md` (the panel, the service and the SDK client were wired
there). This round makes the panel classify items through the preview registry, shows the URL of a titled item, and
confirms the Files Space hook. Thumbnails are still not wired (see the gaps).

## What is now wired

- **Kind by registry.** `springboard/src/shell/preview_panel.rs` `registry()` registers four providers with
  `bifrost_ui::preview::Registry`: pictures (`public.image`), text (`public.text`), PDF (`com.adobe.pdf`) and audio
  (`public.audio`). `kind_for(file_name)` asks the registry, so a subtype takes its ancestor's provider (Markdown and Rust
  source are text, PNG and JPEG are pictures, MP3 is audio). The extension list is gone.
- **PDF and audio items.** The body says which app reads or plays the item: "PDF document. Open it in Books to read it."
  and "Audio file. Open it in Music to play it." The panel does not decode or play them. Other kinds keep the "No preview"
  line.
- **Title and URL.** The headline is the title (or the URL when there is no title). When the item has its own title, its
  URL is drawn in secondary footnote grey in a strip under the body (`Geometry::caption`). The body shrinks by that strip.
- **Several items.** Arrow keys, the chevrons and the owner's `step` move through the items with no wrap; the counter reads
  `n of m` (already wired; the tests still cover it).
- **Files Space (already wired, unchanged).** `springboard/apps/files/src/system.rs` `Files::quick_look` sends the selection
  through `quick_look::show` (`Effect::ShowPreview`), starting at the last selected row. Space again closes it. The
  panel's `step` events move the Files selection (`follow_preview`), and its `closed` event clears Files' state. Files
  uses its in-app preview only if the shell refuses the call (`Waiting::Preview`).

## How a user reaches it

- In Files, select one or more items and press Space (or use the Preview item). The panel opens on the last selected item.
- Left and Right arrows, or the chevrons, step through the selection. The counter reads `n of m`. The Files selection
  follows the panel.
- Esc, Space or the close button hide it. Files receives `closed`.
- Any other app can open the panel through the `quick-look` service or `ToShell::Preview` (see `wire-svc-quicklook.md`).

## How to verify on the test PC (Midget)

Midget is the only test machine (no VM).

1. Build and push the fast shell: `cargo build --offline --profile fast --bin shell --features desktop-wayland` (in
   `springboard/`), then `tools/midget-push-shell.sh --no-build shot.png`.
2. On Midget, open Files and make a folder with `notes.md`, `photo.png`, `paper.pdf` and `song.mp3`. Select all four.
3. Press Space: `bifrost pc midget screen /tmp/ql1.png`. The card opens on the last selected item and the counter reads
   `4 of 4`. The title is the file name, and the URL strip shows its `file://` path.
4. `bifrost pc midget key left`, then `bifrost pc midget screen`: the counter goes down by one and the body changes to
   that item (a picture, text lines, or the PDF or audio line). Repeat until `1 of 4`; Left at `1 of 4` does not move.
5. Press Space: the card is gone, and the Files selection is unchanged.
6. Check Light and Dark (`bifrost pc midget appearance light|dark`): the card, the URL strip and the chevrons follow the
   appearance.

Tests (run by the lead): `cargo test -p springboard preview_panel` (kinds by registry, PDF and audio bodies, geometry
including the caption strip), and `cargo test -p springboard services::quick_look`.

## REA evidence (official documentation)

Reused from `wire-svc-quicklook.md`; not fetched again this round.

- `ev_053580a3b907fd4d4e73ae530fae08c6434f6a5e8c7ceda6695abe5d67d9af7f`: the Quick Look collection page
  (`https://developer.apple.com/documentation/quicklook`).
- `ev_e355715852131d433590760283741dd940ab93617c907715eed43f1977c603e7`: `documentation/quicklook/qlpreviewpanel` was not
  found at that path. Not used as a source for behaviour.

## Remaining gaps

- **Thumbnails are not drawn.** The panel has no filmstrip or per-item thumbnail, and it does not call
  `ThumbnailCache::best_representation` (the SDK model in `springboard/sdk/src/peek.rs` is for apps; the shell does not
  depend on `bifrost_app`). The shell decodes each picture again on each step.
- **PDF and audio are not rendered.** The panel names the app; it shows no page text, no PDF page view and no play button
  (Files' in-app preview does have a Play button for audio).
- Only `file://` URLs are read. Other schemes show "No preview for this item."
- The panel is drawn on the desktop only (`draw_desk`). A full-screen app does not show it.
- `previewItemDisplayState` (Loading, Failed) is not carried on the wire.
- Events are sent from the service tick, so they arrive on the next shell tick.

## Matrix rows (not edited here; the lead updates `docs/parity-matrix/data/`)

| Slug | Feature (as in the matrix) | BifrOSt path |
|---|---|---|
| quick-look | Preview panel (floating panel, items, index, Space toggle) | springboard/src/shell/preview_panel.rs; springboard/apps/files/src/system.rs (Files Space) |
| quick-look | In-app preview of common file types | springboard/src/shell/preview_panel.rs (PDF and audio kinds by registry) |
| quicklook-ui | Preview panel (QLPreviewPanel) | springboard/src/shell/preview_panel.rs |
| quartz | Quick Look preview panel (QLPreviewPanel, QLPreviewItem, QLPreviewPanelDataSource) | springboard/src/shell/preview_panel.rs |
| quick-look-thumbnailing | Quick Look panel showing thumbnails and previews | springboard/src/shell/preview_panel.rs (no thumbnails yet; stays partial) |
| quick-look-thumbnailing | Thumbnail cache and memory-pressure purge | not wired in this round (stays as it is) |
