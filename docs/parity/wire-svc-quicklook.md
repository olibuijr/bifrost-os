# Wire: preview panel (service `quick-look`)

Branch `wire/svc-quicklook`. Wires the Preview models (`bifrost_app::peek`, `PreviewPanel`) into the desktop. Before this,
the shell decoded `ToShell::Preview` (0x1e0) and `ToShell::PreviewClose` (0x1e1) and dropped them.

## What is now wired

- **Panel.** The shell draws a floating preview card over the desktop (above the windows and the Dock, at the menus'
  layer). It shows one item: a picture (decoded and fitted, at most 16 MiB on disk), the first 24 lines of a text file
  (at most 16 KiB read), or a line saying there is no preview. Title, `n of N` counter, close button, chevron buttons.
  Code: `springboard/src/shell/preview_panel.rs` (model `Panel`, pure `geometry`, `draw_quick_look`, pointer and key
  handlers).
- **Apps reach it two ways.** `PreviewPanel::make_key_and_order_front` (`ToShell::Preview`), which the shell now handles
  in `apphost.rs` `rich_msg`, and the new `quick-look` service (`springboard/src/shell/services/quick_look.rs`) with
  methods `show`, `close`, `state`, `step`. The SDK client is `springboard/sdk/src/quick_look.rs`
  (`bifrost_app::quick_look`): `show`, `close`, `state`, `step`, `parse_state`, `event`.
- **User input.** Left and Right arrows and the chevrons step (no wrapping). Esc, Space and the close button hide it.
  A press or release inside the card is taken by the panel; the desktop outside the card keeps working.
- **Events to the owner.** `closed` (user closed it) and `index` `{index}` (user stepped). Only the app that opened the
  panel receives them (`App::service_event`, service name `quick-look`).
- **Owner rule.** `close` and `step` from the service, and `PreviewClose` from the app, only act when the calling app is
  the one that opened the panel.

## How a user reaches it

Any app on the desktop that sends a preview. In a window on the desktop: the app calls `PreviewPanel` (or the service).
The panel appears in the centre of the screen. Keys: Left/Right, Esc or Space.

## How to verify on the test PC (Midget)

Midget is the only test machine (no VM). Build the fast shell, push it, then drive the panel:

1. `cargo build --offline --profile fast --bin shell --features desktop-wayland` (in `springboard/`), then
   `tools/midget-push-shell.sh --no-build shot.png`.
2. Open an app that previews, or have an SDK test app call `bifrost_app::quick_look::show(ctx, &items, 0)` with two
   `file://` URLs (one `.png`, one `.txt`).
3. `bifrost pc midget screen /tmp/ql.png`: the card is centred; the title, counter and body match the item.
4. `bifrost pc midget key right`, then `bifrost pc midget screen`: the counter reads `2 of 2`.
5. `bifrost pc midget key esc`: the card is gone; the owner app receives `closed` (check with `bifrost pc control call`).
6. Check Light and Dark (`bifrost pc midget appearance light|dark`): card, text and chevrons follow the appearance.

Unit tests (run by the lead, not in this round): `cargo test -p springboard preview_panel` (model, geometry, text
loading, event queue), `cargo test -p springboard services::quick_look`, and `cargo test -p bifrost-app quick_look`
(SDK request bodies, state parsing, event parsing).

## REA evidence (official documentation)

- `ev_053580a3b907fd4d4e73ae530fae08c6434f6a5e8c7ceda6695abe5d67d9af7f`: "Quick Look" collection page
  (`https://developer.apple.com/documentation/quicklook`). Lists QLPreviewController, QLPreviewItem and the data-based
  preview classes. The panel behaviour here follows the controller's in-app preview role.
- `ev_e355715852131d433590760283741dd940ab93617c907715eed43f1977c603e7`: a fetch of
  `documentation/quicklook/qlpreviewpanel` returned "page can't be found" (the official page is not at that path). It is
  not used as a source for behaviour.

## Remaining gaps

- The panel is drawn only on the desktop (`draw_desk`). An app that runs full screen (not in a desktop window) does not
  show it, and its key and pointer events do not reach it.
- GPU presenter partial frames: a change sets a full-frame damage (`Panel::take_dirty`), and the panel is kept out of the
  partial-repaint path while visible. The GPU region bookkeeping (`desk_regions_now`) does not list the card, so a
  GPU-presented change may need a full repaint. Verify on Midget.
- No thumbnail generator, no thumbnail cache use, and no PDF, video or audio preview (those are in Files only).
- `previewItemDisplayState` (Loading, Failed) is not carried on the wire; the panel shows the loaded body only.
- Only `file://` URLs are read. Other schemes show "No preview for this item."
- No markup button, no transition source frames, no full-screen mode, and no data-based preview extension.
- Events are sent from the service tick (`services_tick`), so they arrive on the next shell tick.
